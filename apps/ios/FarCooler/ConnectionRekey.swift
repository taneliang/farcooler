import Foundation
import SwiftUI

// "Make a New Key" on the iPhone (ov-569). The steps are AgentKit's
// (`PhoneRekey`); this file is what only the app can do: hold the keys in the
// Keychain, pick which one reaches a runner, and move a runner's line over a
// connection the old key opened (`client.rekey`).

/// The runners (by id) still holding only the old key. In UserDefaults, which
/// is safe to read from anywhere, since a connection asks before it dials.
enum RekeyMarks {
    private static let key = "rekey.runnersOnPrevious"

    static var onPrevious: Set<String> {
        get { Set(UserDefaults.standard.stringArray(forKey: key) ?? []) }
        set { UserDefaults.standard.set(newValue.sorted(), forKey: key) }
    }
}

/// The Keychain's three slots, as AgentKit sees them: public halves only.
@MainActor
final class KeychainKeySlots: PhoneKeySlots {
    static let shared = KeychainKeySlots()

    var currentPublicKey: String? { Identity.publicKey }
    var nextPublicKey: String? { Identity.nextPrivateKey.flatMap(Identity.publicKey(of:)) }
    var previousPublicKey: String? { Identity.previousPrivateKey.flatMap(Identity.publicKey(of:)) }
    func makeNext() -> String? { Identity.makeNext() }
    func promote() -> Bool { Identity.promote() }
    func forgetPrevious() { Identity.forgetPrevious() }
    func proveNext(account: String, challenge: String) -> DeviceKeyProof? {
        Identity.nextRegistrationProof(account: account, challenge: challenge)
    }
    func certifyByCurrent(account: String, keyA: String) -> String? {
        Identity.certificate(account: account, scannedKeyA: keyA)
    }
    var runnersOnPrevious: Set<String> {
        get { RekeyMarks.onPrevious }
        set { RekeyMarks.onPrevious = newValue }
    }

    /// Give AgentKit the slots, the runners and the way to move one. Once, at
    /// launch.
    static func install(runners: RunnerStore) {
        let rekey = PhoneRekey.shared
        rekey.keys = shared
        rekey.label = { UIDevice.current.name }
        rekey.runners = { runners.hosts.map { RekeyRunner(id: $0.id.uuidString, name: $0.label) } }
        rekey.move = { @MainActor runner in
            guard let id = UUID(uuidString: runner.id), let connection = Connection.live(for: id) else {
                return .notReached
            }
            return await connection.moveToNewKey()
        }
        PhoneRekey.codeView = { AnyView(JoinView(runners: runners)) }
    }
}

extension Connection {
    /// Connect with the key this runner gets, and if it refuses that key, try
    /// the phone's other one once. A runner that moved when its answer was
    /// lost refuses the old key; one that never moved refuses the new one.
    /// Either way, the key that worked says where the runner is now.
    func connectWithEitherKey(host: Runner, key: String, nodeKey: String?) async throws {
        let config = { (key: String) in
            host.config(privateKey: key, nodeKey: nodeKey, derpMap: Account.shared.derpMap)
        }
        do {
            _ = try await core.connect(config: config(key))
        } catch {
            let id = host.id.uuidString
            let hasPrevious = Identity.previousPrivateKey != nil
            let tried = PhoneKeyChoice.forRunner(id, onPrevious: RekeyMarks.onPrevious, hasPrevious: hasPrevious)
            guard Self.refusedTheKey(error), let other = tried.alternate(hasPrevious: hasPrevious),
                let otherKey = Identity.privateKey(for: host.id, choice: other)
            else { throw error }
            _ = try await core.connect(config: config(otherKey))
            if other == .current {
                RekeyMarks.onPrevious.remove(id)
            } else {
                RekeyMarks.onPrevious.insert(id)
            }
        }
    }

    /// After a connection comes up: a runner still on the old key moves now.
    func moveToNewKeyIfNeeded(for host: Runner) {
        let id = host.id.uuidString
        guard RekeyMarks.onPrevious.contains(id), Identity.previousPrivateKey != nil else { return }
        Task { await PhoneRekey.shared.moveIfNeeded(runnerID: id) }
    }

    /// Move this runner's line to the key in use, over this connection, which
    /// the old key opened. `client.rekey` rewrites only the line this
    /// connection authenticated as.
    func moveToNewKey() async -> RunnerMove {
        guard phase == .connected else { return .notReached }
        await loadDaemonBuild()
        guard let daemon else { return .notReached }
        guard daemon.can(.rekey) else { return .needsUpdate }
        guard let proof = Identity.rekeyProof() else { return .refused }
        do {
            _ = try await core.call("client.rekey", proof)
            return .moved
        } catch {
            // A dropped link isn't a refusal: the next connection finds out
            // which key the runner takes.
            return phase == .connected ? .refused : .notReached
        }
    }

    /// Whether a connect failed because the runner doesn't know the key.
    static func refusedTheKey(_ error: Error) -> Bool {
        guard let core = error as? ClientCore.CoreError, case let .unreached(_, trouble, tunnel, _) = core else {
            return false
        }
        return RunnerTrouble(trouble: trouble, tunnel: tunnel) == .keyRejected
    }
}
