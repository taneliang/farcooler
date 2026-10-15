import Foundation
import Testing

@testable import AgentKit

/// A phone's "Make a New Key" (ov-569), with fake key slots, a scripted relay
/// and fake runners.
@MainActor
struct PhoneRekeyTests {
    static let old = "ssh-ed25519 AAAAold farcooler-iPhone"
    static let new = "ssh-ed25519 AAAAnew farcooler-iPhone"

    /// The Keychain, as three slots of public keys.
    final class Slots: PhoneKeySlots {
        var currentPublicKey: String? = PhoneRekeyTests.old
        var nextPublicKey: String?
        var previousPublicKey: String?
        var runnersOnPrevious: Set<String> = []
        var refusesToMake = false
        var proved: [(account: String, challenge: String)] = []
        var certified: [(account: String, keyA: String)] = []

        func makeNext() -> String? {
            guard !refusesToMake else { return nil }
            nextPublicKey = PhoneRekeyTests.new
            return nextPublicKey
        }
        /// The marks as they stood when the new key was put in use.
        var marksAtPromote: Set<String>?
        func promote() -> Bool {
            marksAtPromote = runnersOnPrevious
            guard let next = nextPublicKey else { return false }
            previousPublicKey = currentPublicKey
            currentPublicKey = next
            nextPublicKey = nil
            return true
        }
        func forgetPrevious() { previousPublicKey = nil }
        func proveNext(account: String, challenge: String) -> DeviceKeyProof? {
            proved.append((account, challenge))
            return nextPublicKey.map { DeviceKeyProof(keyA: $0, proofA: "proof-by-next") }
        }
        func certifyByCurrent(account: String, keyA: String) -> String? {
            certified.append((account, keyA))
            return "certificate-by-current"
        }
    }

    /// A relay holding the account's device rows, answering each route.
    @MainActor final class Relay {
        var rows: [[String: Any]]
        var registerState = "pending"
        var registerStatus = 200
        private(set) var sent: [(path: String, body: [String: Any])] = []

        init(rows: [[String: Any]]) { self.rows = rows }

        var paths: [String] { sent.map { $0.path.replacingOccurrences(of: "/v1/directory/", with: "") } }
        func body(_ path: String) -> [String: Any]? { sent.last { $0.path.hasSuffix(path) }?.body }

        var client: DirectoryClient {
            DirectoryClient { path, body in
                self.sent.append((path, body))
                switch path {
                case "/v1/directory/devices/challenge":
                    return .success(DirectoryReply(status: 200, body: ["challenge": String(repeating: "a", count: 64)]))
                case "/v1/directory/devices/register":
                    guard self.registerStatus == 200 else {
                        return .success(DirectoryReply(status: self.registerStatus, body: ["error": "nope"]))
                    }
                    self.rows.append(["id": "d-new", "label": "iPhone", "keyA": PhoneRekeyTests.new, "state": self.registerState])
                    return .success(DirectoryReply(status: 200, body: ["device": "d-new", "state": self.registerState]))
                case "/v1/directory/me":
                    return .success(DirectoryReply(status: 200, body: ["devices": self.rows]))
                case "/v1/directory/devices/certify":
                    self.rows = self.rows.map { row in
                        guard row["id"] as? String == "d-new" else { return row }
                        var row = row
                        row["state"] = "certified"
                        row["certifiedBy"] = "d-old"
                        return row
                    }
                    return .success(DirectoryReply(status: 200, body: ["device": "d-new", "state": "certified"]))
                case "/v1/directory/devices/revoke":
                    return .success(DirectoryReply(status: 200, body: ["ok": true]))
                default:
                    return .failure(.endpointMissing)
                }
            }
        }
    }

    static func row(_ id: String, _ keyA: String, _ state: String, label: String = "iPhone") -> [String: Any] {
        ["id": id, "label": label, "keyA": keyA, "state": state]
    }

    static let runners = [RekeyRunner(id: "r1", name: "studio"), RekeyRunner(id: "r2", name: "build-box")]

    func rekey(
        _ slots: Slots, _ relay: Relay?, moves: [String: RunnerMove] = [:], moved: Ref<[String]> = Ref([])
    ) -> PhoneRekey {
        let rekey = PhoneRekey()
        rekey.keys = slots
        rekey.signedIn = { relay.map { ("user_bob", $0.client) } }
        rekey.label = { "iPhone" }
        rekey.runners = { Self.runners }
        rekey.move = { runner in
            moved.value.append(runner.id)
            return moves[runner.id] ?? .moved
        }
        return rekey
    }

    final class Ref<T> {
        var value: T
        init(_ value: T) { self.value = value }
    }

    @Test("A phone whose key was revoked makes a new key, registers it, moves its runners, and waits to be scanned")
    func revokedPhone() async {
        let slots = Slots()
        let relay = Relay(rows: [Self.row("d-mac", "ssh-ed25519 AAAAmac", "trusted"), Self.row("d-old", Self.old, "revoked")])
        let moved = Ref<[String]>([])
        let rekey = rekey(slots, relay, moved: moved)
        await rekey.run()

        #expect(rekey.stage == .finished)
        #expect(slots.currentPublicKey == Self.new, "the phone still connects with the old key")
        #expect(slots.previousPublicKey == nil, "the old key was kept with every runner moved")
        #expect(slots.proved.map(\.account) == ["user_bob"])
        #expect(relay.body("devices/register")?["keyA"] as? String == Self.new)
        #expect(rekey.registered == .pending)
        // A removed key vouches for nothing, so nothing was signed with it,
        // and the person scans the new code from the Mac.
        #expect(slots.certified.isEmpty)
        #expect(rekey.needsScan)
        #expect(moved.value == ["r1", "r2"])
        // Already removed, so there's nothing to revoke.
        #expect(!relay.paths.contains("devices/revoke"))
        #expect(rekey.oldRemoved)
    }

    @Test("A key that is still confirmed confirms its replacement, and hands it its place when removed")
    func confirmedPhone() async {
        let slots = Slots()
        let relay = Relay(rows: [Self.row("d-old", Self.old, "trusted"), Self.row("d-mac", "ssh-ed25519 AAAAmac", "certified")])
        let rekey = rekey(slots, relay)
        await rekey.run()

        #expect(rekey.stage == .finished)
        #expect(slots.certified.map(\.keyA) == [Self.new], "the old key didn't certify the new one")
        #expect(relay.body("devices/certify")?["keyA"] as? String == Self.new)
        #expect(!rekey.needsScan)
        #expect(relay.body("devices/revoke")?["device"] as? String == "d-old")
        #expect(relay.body("devices/revoke")?["successor"] as? String == "d-new")
        // Certified before the key was put to use: the old key signed it.
        let certify = relay.paths.firstIndex(of: "devices/certify")!
        let revoke = relay.paths.firstIndex(of: "devices/revoke")!
        #expect(certify < revoke)
    }

    @Test("The old key's row is found by the key, not by a label the relay could reuse")
    func foundByKey() async {
        let slots = Slots()
        // A confirmed row named like this phone, with someone else's key.
        let relay = Relay(rows: [Self.row("d-fake", "ssh-ed25519 AAAAfake", "trusted", label: "iPhone"),
                                 Self.row("d-old", Self.old, "pending")])
        let rekey = rekey(slots, relay)
        await rekey.run()
        #expect(slots.certified.isEmpty, "a certificate was signed on the strength of another device's row")
        #expect(rekey.needsScan)
        // Removed plainly: the pending old key confirmed nothing.
        #expect(relay.body("devices/revoke")?["device"] as? String == "d-old")
        #expect(relay.body("devices/revoke")?["successor"] == nil)
    }

    @Test("A runner the phone can't reach keeps the old key until the phone reaches it")
    func unreachedRunner() async {
        let slots = Slots()
        let relay = Relay(rows: [Self.row("d-old", Self.old, "trusted")])
        let moved = Ref<[String]>([])
        let rekey = rekey(slots, relay, moves: ["r2": .notReached], moved: moved)
        await rekey.run()

        #expect(rekey.stage == .finished)
        #expect(slots.runnersOnPrevious == ["r2"])
        #expect(slots.previousPublicKey == Self.old, "the old key was deleted while a runner still needs it")
        #expect(rekey.runnersLeft.map(\.id) == ["r2"])
        #expect(PhoneKeyChoice.forRunner("r2", onPrevious: slots.runnersOnPrevious, hasPrevious: true) == .previous)
        #expect(PhoneKeyChoice.forRunner("r1", onPrevious: slots.runnersOnPrevious, hasPrevious: true) == .current)

        // Later, the phone reaches it with the old key.
        rekey.move = { _ in .moved }
        await rekey.moveIfNeeded(runnerID: "r2")
        #expect(slots.runnersOnPrevious.isEmpty)
        #expect(slots.previousPublicKey == nil)
        #expect(rekey.moves["r2"] == .moved)
    }

    @Test("A key store that won't make a key changes nothing")
    func keyStoreRefuses() async {
        let slots = Slots()
        slots.refusesToMake = true
        let relay = Relay(rows: [])
        let moved = Ref<[String]>([])
        let rekey = rekey(slots, relay, moved: moved)
        await rekey.run()
        #expect(rekey.stage == .failed(.keyStore))
        #expect(slots.currentPublicKey == Self.old)
        #expect(relay.sent.isEmpty)
        #expect(moved.value.isEmpty)
    }

    @Test("A relay that won't take the new key leaves the old one in use, and a retry reuses the new key")
    func registrationRefused() async {
        let slots = Slots()
        let relay = Relay(rows: [Self.row("d-old", Self.old, "trusted")])
        relay.registerStatus = 500
        let moved = Ref<[String]>([])
        let rekey = rekey(slots, relay, moved: moved)
        await rekey.run()
        guard case .failed(.register) = rekey.stage else {
            Issue.record("expected a registration failure, got \(rekey.stage)")
            return
        }
        #expect(slots.currentPublicKey == Self.old)
        #expect(slots.nextPublicKey == Self.new)
        #expect(moved.value.isEmpty, "runners moved to a key the account doesn't have")

        relay.registerStatus = 200
        slots.refusesToMake = true  // a second key would be refused
        await rekey.run()
        #expect(rekey.stage == .finished)
        #expect(slots.currentPublicKey == Self.new)
    }

    @Test("Signed out, the runners still move, and nothing is asked of the relay")
    func signedOut() async {
        let slots = Slots()
        let moved = Ref<[String]>([])
        let rekey = rekey(slots, nil, moved: moved)
        await rekey.run()
        #expect(rekey.stage == .finished)
        #expect(slots.currentPublicKey == Self.new)
        #expect(moved.value == ["r1", "r2"])
        #expect(slots.proved.isEmpty)
    }

    @Test("After a runner refuses one key, the other is tried once, and only while the phone has both")
    func alternateKey() {
        #expect(PhoneKeyChoice.current.alternate(hasPrevious: true) == .previous)
        #expect(PhoneKeyChoice.previous.alternate(hasPrevious: true) == .current)
        #expect(PhoneKeyChoice.current.alternate(hasPrevious: false) == nil)
        #expect(PhoneKeyChoice.forRunner("r1", onPrevious: ["r1"], hasPrevious: false) == .current)
    }

    @Test("Forgetting the old key gives up on the runners left on it")
    func forgetOld() async {
        let slots = Slots()
        let rekey = rekey(slots, nil, moves: ["r1": .needsUpdate, "r2": .refused])
        await rekey.run()
        #expect(slots.runnersOnPrevious == ["r1", "r2"])
        #expect(rekey.isUnderway)
        rekey.forgetOldKey()
        #expect(slots.previousPublicKey == nil)
        #expect(slots.runnersOnPrevious.isEmpty)
        #expect(!rekey.isUnderway)
    }

    @Test("Settings says where each step is, in plain words, runner by runner")
    func progressLines() async {
        let slots = Slots()
        let relay = Relay(rows: [Self.row("d-old", Self.old, "revoked")])
        let rekey = rekey(slots, relay, moves: ["r2": .needsUpdate])
        await rekey.run()
        let lines = rekey.progress(signedIn: true)
        #expect(lines.map(\.id) == ["key", "account", "runner-r1", "runner-r2", "old"])
        #expect(lines.map(\.done) == [true, false, true, false, true])
        #expect(lines[1].detail.contains("scan it from one of your confirmed devices"))
        #expect(lines[3].title == "build-box")
        #expect(lines[3].detail == RunnerMove.needsUpdate.detail)
        // Signed out: no account steps, and a way to add the key later.
        let signedOut = rekey.progress(signedIn: false)
        #expect(signedOut.map(\.id) == ["key", "account", "runner-r1", "runner-r2"])
        #expect(signedOut[1].detail == "Sign in to add the new key to your account.")
        #expect(PhoneRekey.invitation(removed: true).contains("removed from your account"))
    }

    @Test("Every runner is marked before the new key is put in use, so stopping in between loses no old key")
    func marksBeforePromote() async {
        let slots = Slots()
        let rekey = rekey(slots, nil, moves: ["r1": .notReached, "r2": .notReached])
        await rekey.run()
        #expect(slots.marksAtPromote == ["r1", "r2"], "the new key was in use before the runners were marked")

        // Stopped right after promoting: a fresh run (Continue) finds both runners
        // still on the old key, and keeps it.
        let resumed = PhoneRekeyTests().rekey(slots, nil, moves: ["r1": .notReached, "r2": .notReached])
        await resumed.run()
        #expect(slots.previousPublicKey == Self.old)
        #expect(slots.runnersOnPrevious == ["r1", "r2"])
    }
}
