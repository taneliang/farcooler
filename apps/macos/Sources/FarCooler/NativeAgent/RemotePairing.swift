import AgentKit
import Foundation

/// How the Mac reaches a remote runner's conversation (ov-408): its own key
/// (`ConversationKey`), paired with the runner the way a phone is, and one
/// client-core session per runner.
///
/// **Pairing.** A phone's key is written into a runner's `authorized_keys` by
/// a device that already reaches it, through `client.enroll`. The Mac already
/// reaches every runner it lists, over the owner's own ssh, so it asks for its
/// own line the same way, through the CLI:
///
/// ```
/// farcooler --json --runner <target> client enroll --key <public> \
///     --label "<Mac> Conversation View" --client-id <derived> --scope control
/// ```
///
/// The daemon writes the line (restricted, a forced command, `control`), as it
/// does for a phone. That grants the app's connection nothing the Mac doesn't
/// already have: the owner's ssh on that runner is a shell.
///
/// **Revocation is never undone by itself.** A runner is paired automatically
/// only when this Mac has no record of it. A runner that refuses a key this
/// Mac paired was revoked (`client revoke`), and one the person unpaired here
/// stays unpaired. Both show the terminal until Pair Again in Settings.
///
/// **The owner's ssh setup doesn't change.** `RemoteReach` reads the config
/// and `known_hosts`; nothing here writes a file on this Mac but the
/// Keychain item and this record.
@MainActor
final class RemotePairing: ObservableObject {
    static let shared = RemotePairing()

    /// What one runner's pairing is, for Settings.
    enum State: Equatable, Sendable {
        /// This Mac's key is on the runner and the view reads it.
        case paired
        /// The runner refused a key this Mac had paired: someone removed it.
        case removed
        /// The person unpaired it here.
        case unpaired
        /// Can't be reached this way, in a sentence that says why.
        case unavailable(String)
    }

    /// What a connect came to.
    enum Outcome: Sendable {
        case connected(RunnerCore, Set<String>)
        /// Not until a person does something; the state says what. Not
        /// retried.
        case unavailable
        /// Didn't answer this time; retried with backoff.
        case unreachable
    }

    /// Each remote runner's state, by target, as of the last try.
    @Published private(set) var states: [String: State] = [:]

    /// How the CLI runs. The real one in the app; a test passes its own.
    var cli: ([String]) async -> CLI.Result = { await CLI.run($0) }
    /// Where a target is. `ssh -G` and `known_hosts` in the app.
    var resolve: (String) async -> Result<RemoteReach, RemoteReach.Refusal> = { target in
        await RemoteReach.resolve(target, ssh: RemoteReach.ssh())
    }
    /// One connect through the client core: the runner, the private key,
    /// the host key to require (none: fail with the one shown). A test
    /// passes its own.
    var dial: (RemoteReach, String, String?) async throws -> (RunnerCore, Set<String>) = RemotePairing.dialCore

    nonisolated static func dialCore(_ reach: RemoteReach, _ privateKey: String, _ pin: String?) async throws -> (RunnerCore, Set<String>) {
        let core = RunnerCore()
        return (core, try await core.connect(reach: reach, privateKey: privateKey, fingerprint: pin))
    }
    let key: ConversationKey
    private let defaults: UserDefaults
    /// The host key each runner was found to present, checked against
    /// `known_hosts`, for this run of the app.
    private var pins: [String: String] = [:]
    /// Runners whose projector this run already turned on.
    private var projectorAsked: Set<String> = []

    static let recordKey = "nativeAgent.remotePairings"

    /// The record: target to the client id it was paired under, or to what
    /// stopped it (`removed`, `unpaired`). Labels, no secrets.
    private var record: [String: String] {
        get { defaults.dictionary(forKey: Self.recordKey) as? [String: String] ?? [:] }
        set { defaults.set(newValue, forKey: Self.recordKey) }
    }

    init(key: ConversationKey = .shared, defaults: UserDefaults = .standard) {
        self.key = key
        self.defaults = defaults
        for (target, value) in record {
            states[target] = value == "removed" ? .removed : value == "unpaired" ? .unpaired : .paired
        }
    }

    /// The label the runner's device list shows for this Mac's line.
    static var label: String { "\(thisMacName) Conversation View" }

    /// Reach `target`'s runner, pairing first where this Mac never has.
    func connect(target: String) async -> Outcome {
        let reach: RemoteReach
        switch await resolve(target) {
        case .failure(let refusal): return unavailable(target, refusal.sentence)
        case .success(let found): reach = found
        }
        guard let privateKey = key.privateKey(), let clientID = key.clientID, let publicKey = key.publicKey else {
            return unavailable(target, "Far Cooler couldn’t keep this Mac’s key in the Keychain.")
        }
        let recorded = record[target]
        if recorded == "unpaired" {
            states[target] = .unpaired
            return .unavailable
        }
        // Paired under THIS key, or revoked: a record of a key this Mac no
        // longer holds says nothing about the one it holds now.
        let wasPaired = recorded == clientID || recorded == "removed"

        let pin: String
        switch await hostKey(target, reach: reach, privateKey: privateKey) {
        case .pinned(let found): pin = found
        case .mismatch:
            return unavailable(target, "This runner’s host key doesn’t match the one ssh knows, so Far Cooler won’t connect to it.")
        case .failed: return .unreachable
        }

        switch await open(reach, privateKey: privateKey, pin: pin) {
        case .connected(let core, let offered):
            return await settle(target, core, offered, reach: reach, privateKey: privateKey, pin: pin, clientID: clientID)
        case .rejected:
            // Revoked: a key this Mac paired, refused. Never re-paired here.
            if wasPaired {
                record[target] = "removed"
                states[target] = .removed
                return .unavailable
            }
            let enrolled = await cli([
                "--json", "--runner", target, "client", "enroll", "--key", publicKey, "--label", Self.label,
                "--client-id", clientID, "--scope", "control",
            ])
            guard enrolled.ok else {
                return unavailable(target, "This runner didn’t take this Mac’s key. Update Far Cooler on it and try again.")
            }
            record[target] = clientID
            guard case .connected(let core, let offered) = await open(reach, privateKey: privateKey, pin: pin) else { return .unreachable }
            return await settle(target, core, offered, reach: reach, privateKey: privateKey, pin: pin, clientID: clientID)
        case .failed:
            return .unreachable
        }
    }

    /// Remove this Mac's key from `target`'s runner (`client revoke`), and
    /// pair it again only when asked. Nil when it's done; a sentence when
    /// the runner didn't answer.
    func unpair(target: String) async -> String? {
        guard let clientID = key.clientID else { return "Far Cooler couldn’t read this Mac’s key." }
        let revoked = await cli(["--json", "--runner", target, "client", "revoke", clientID])
        guard revoked.ok else { return "Far Cooler couldn’t reach this runner to unpair it. Try again when it’s online." }
        record[target] = "unpaired"
        states[target] = .unpaired
        pins[target] = nil
        return nil
    }

    /// Forget why `target` stopped, so the next connect pairs it again.
    func allowPairing(target: String) {
        record[target] = nil
        states[target] = nil
    }

    // MARK: - Steps

    private enum Opened {
        case connected(RunnerCore, Set<String>)
        case rejected
        case failed
    }

    private func open(_ reach: RemoteReach, privateKey: String, pin: String) async -> Opened {
        do {
            let (core, offered) = try await dial(reach, privateKey, pin)
            return .connected(core, offered)
        } catch RunnerCore.Failure.unconnected(_, trouble: "key_rejected", _) {
            return .rejected
        } catch {
            return .failed
        }
    }

    private enum Pin {
        case pinned(String)
        /// The runner showed a key `known_hosts` doesn't hold for it.
        case mismatch
        case failed
    }

    /// The host key `target` presents, when the owner's `known_hosts` holds
    /// it. The core shows the key and refuses before signing in, so the
    /// private key is never offered to a host that isn't checked.
    private func hostKey(_ target: String, reach: RemoteReach, privateKey: String) async -> Pin {
        if let pinned = pins[target], reach.knownKeys.contains(pinned) { return .pinned(pinned) }
        do {
            _ = try await dial(reach, privateKey, nil)
        } catch RunnerCore.Failure.unconnected(_, trouble: "host_key_unknown", let fingerprint?) {
            guard reach.knownKeys.contains(fingerprint) else { return .mismatch }
            pins[target] = fingerprint
            return .pinned(fingerprint)
        } catch {}
        return .failed
    }

    /// Paired and connected; turn the runner's projector on once if its hello
    /// doesn't serve rows, since the setting says it does.
    private func settle(
        _ target: String, _ core: RunnerCore, _ offered: Set<String>, reach: RemoteReach, privateKey: String, pin: String,
        clientID: String
    ) async -> Outcome {
        record[target] = clientID
        states[target] = .paired
        guard !NativeAgents.serves(offered), !projectorAsked.contains(target) else { return .connected(core, offered) }
        projectorAsked.insert(target)
        guard await cli(["--runner", target, "settings", "set-projector", "on"]).ok,
            case .connected(let fresh, let offered) = await open(reach, privateKey: privateKey, pin: pin)
        else { return .connected(core, offered) }
        return .connected(fresh, offered)
    }

    private func unavailable(_ target: String, _ sentence: String) -> Outcome {
        states[target] = .unavailable(sentence)
        return .unavailable
    }
}
