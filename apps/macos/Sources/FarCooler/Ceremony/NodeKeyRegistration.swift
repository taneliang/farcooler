import AgentKit
import Foundation

/// This Mac's tunnel key onto its own line on a runner it was paired with
/// before the tunnel existed, so that runner's tunnel admits it without pairing
/// again (ov-485, `client.set_node_key`).
///
/// **Registered, not switched.** A runner this Mac reaches at an address is
/// usually one the owner added by its `ssh` alias, in their own
/// `~/.ssh/config`, which Far Cooler doesn't rewrite. So the route stays as it
/// is; what changes is that the runner admits this Mac's key, which is what a
/// later grant through Tailcat needs.
///
/// Once per runner and key: remembered after the runner said yes, and tried at
/// most once per launch when it said no (an older runner with no tunnel says no
/// every time, and asking on every reconnect would spawn a process for nothing).
struct NodeKeyRegistration {
    /// On in the app, from its one dialing store; off in a test process, where
    /// a reconnect must not reach the Keychain or spawn the CLI.
    nonisolated(unsafe) static var enabled = false

    typealias Run = @Sendable ([String]) async -> (ok: Bool, output: String)

    /// This Mac's public node key, or nil when it has none to give.
    var key: @Sendable () async -> String? = { await MacNodeIdentity.shared.publicKeyForOffer() }
    var run: Run = {
        let result = await CLI.run($0)
        return (result.ok, result.ok ? result.output : result.said)
    }
    /// Whether `target` is already reached through the tunnel, and so already
    /// carries this key from its grant.
    var tunneled: @Sendable (String) -> Bool = { target in
        SshConfig.entries().contains { $0.alias == target && $0.tunneled }
    }
    var remembered: RegisteredKeys = .shared

    /// What the call did, for the tests.
    enum Outcome: Equatable {
        case registered
        case alreadyRegistered
        case skipped
        case refused
    }

    @discardableResult
    func register(_ target: String) async -> Outcome {
        guard !target.isEmpty, !tunneled(target), let key = await key(), !key.isEmpty else {
            return .skipped
        }
        let mark = "\(target)\n\(key)"
        guard remembered.begin(mark) else { return .alreadyRegistered }
        let answer = await run(["--json", "--runner", target, "client", "set-node-key", "--node-key", key])
        if answer.ok {
            remembered.done(mark)
            return .registered
        }
        return .refused
    }
}

/// Which runner and key pairs were registered, and which were tried this
/// launch. The first survives a relaunch; the second doesn't, on purpose.
final class RegisteredKeys: @unchecked Sendable {
    static let shared = RegisteredKeys(defaultsKey: "registeredNodeKeys.\(AppVersion.channel)")

    private let lock = NSLock()
    private let defaultsKey: String?
    private var tried: Set<String> = []
    private var kept: Set<String>

    /// `defaultsKey` nil keeps everything in memory, for a test.
    init(defaultsKey: String?) {
        self.defaultsKey = defaultsKey
        kept = Set(defaultsKey.flatMap { UserDefaults.standard.stringArray(forKey: $0) } ?? [])
    }

    /// True when `mark` should be tried now: never registered, never tried.
    func begin(_ mark: String) -> Bool {
        lock.withLock { !kept.contains(mark) && tried.insert(mark).inserted }
    }

    func done(_ mark: String) {
        let all = lock.withLock { () -> [String] in
            kept.insert(mark)
            return kept.sorted()
        }
        if let defaultsKey { UserDefaults.standard.set(all, forKey: defaultsKey) }
    }
}
