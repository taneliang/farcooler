import Foundation
import Testing

@testable import AgentKit

/// The trusted device certifying the one it scanned in the same-account
/// ceremony (ov-550), and the account's device keys in Settings (ov-549
/// review M1). The Mac and iPhone ceremonies both call `certifyScanned`.
@MainActor
struct DeviceCertificationTests {
    static let scanned = "ssh-ed25519 AAAAscanned farcooler-Alice’s iPhone"
    static let certificate = "-----BEGIN SSH SIGNATURE-----\nc\n-----END SSH SIGNATURE-----\n"

    /// A relay that answers `devices/certify` from a script, in order, and
    /// records what it was sent.
    @MainActor final class Relay {
        var answers: [DirectoryReply]
        private(set) var sent: [(path: String, body: [String: Any])] = []

        init(_ answers: [DirectoryReply]) { self.answers = answers }

        var client: DirectoryClient {
            DirectoryClient { path, body in
                self.sent.append((path, body))
                guard !self.answers.isEmpty else { return .failure(.endpointMissing) }
                return .success(self.answers.removeFirst())
            }
        }
    }

    static func reply(_ status: Int, _ body: [String: Any] = [:]) -> DirectoryReply {
        DirectoryReply(status: status, body: body)
    }

    /// A certifier signed in as `account`, signing with a recorder, pausing
    /// for no time and recording each pause.
    final class Recorder {
        var signed: [(account: String, keyA: String)] = []
        var paused: [Duration] = []
    }

    func certifier(
        _ relay: Relay, account: String? = "user_alice", recorder: Recorder, signs: Bool = true
    ) -> DeviceCertification {
        let certification = DeviceCertification()
        certification.signedIn = { account.map { ($0, relay.client) } }
        certification.sign = { account, keyA in
            recorder.signed.append((account, keyA))
            return signs ? Self.certificate : nil
        }
        certification.pause = { recorder.paused.append($0) }
        return certification
    }

    @Test("The scanned code's Key A is signed for this device's own account and posted with the certificate")
    func certifiesTheScannedKey() async {
        let relay = Relay([Self.reply(200, ["device": "d-2", "state": "certified"])])
        let recorder = Recorder()
        let outcome = await certifier(relay, recorder: recorder)
            .certifyScanned(account: "user_alice", keyA: Self.scanned)
        #expect(outcome == .certified)
        #expect(recorder.signed.map(\.account) == ["user_alice"])
        #expect(recorder.signed.map(\.keyA) == [Self.scanned])
        #expect(relay.sent.map(\.path) == ["/v1/directory/devices/certify"])
        #expect(relay.sent[0].body["keyA"] as? String == Self.scanned)
        #expect(relay.sent[0].body["certificate"] as? String == Self.certificate)
        #expect(Set(relay.sent[0].body.keys) == ["keyA", "certificate"])
    }

    @Test("A code for an account other than the one signed in here is never signed, nor is one with nobody signed in")
    func neverForAnotherAccount() async {
        for (signedIn, code) in [("user_alice", "user_bob"), (nil, "user_alice"), ("", "")] as [(String?, String)] {
            let relay = Relay([Self.reply(200)])
            let recorder = Recorder()
            let outcome = await certifier(relay, account: signedIn, recorder: recorder)
                .certifyScanned(account: code, keyA: Self.scanned)
            #expect(outcome == .notSigned)
            #expect(recorder.signed.isEmpty)
            #expect(relay.sent.isEmpty)
        }
    }

    @Test("A new device that hasn't registered yet is waited for, then certified")
    func waitsForTheNewDeviceToRegister() async {
        let relay = Relay([Self.reply(404), Self.reply(404), Self.reply(200)])
        let recorder = Recorder()
        let outcome = await certifier(relay, recorder: recorder)
            .certifyScanned(account: "user_alice", keyA: Self.scanned)
        #expect(outcome == .certified)
        #expect(recorder.paused == [.seconds(2), .seconds(5)])
        #expect(relay.sent.count == 3)
        // Signed once: the same certificate is posted each time.
        #expect(recorder.signed.count == 1)
    }

    @Test("Waiting gives up after the last retry, and stops if the account changes meanwhile")
    func givesUp() async {
        let relay = Relay(Array(repeating: Self.reply(404), count: 10))
        let recorder = Recorder()
        let certification = certifier(relay, recorder: recorder)
        #expect(await certification.certifyScanned(account: "user_alice", keyA: Self.scanned) == .notRegistered)
        #expect(relay.sent.count == 1 + certification.waits.count)

        let switched = Relay(Array(repeating: Self.reply(404), count: 10))
        let other = certifier(switched, recorder: Recorder())
        other.pause = { _ in other.signedIn = { ("user_bob", switched.client) } }
        #expect(await other.certifyScanned(account: "user_alice", keyA: Self.scanned) == .notSigned)
        #expect(switched.sent.count == 1)
    }

    @Test("Already confirmed, refused, unsigned and failed are told apart")
    func outcomes() async {
        let cases: [(DirectoryReply, DeviceCertificationOutcome)] = [
            (Self.reply(409, ["error": "state", "state": "certified"]), .alreadyConfirmed),
            (Self.reply(409, ["error": "state", "state": "trusted"]), .alreadyConfirmed),
            (Self.reply(409, ["error": "state", "state": "revoked"]), .refused),
            (Self.reply(403, ["error": "certificate"]), .refused),
            (Self.reply(500), .failed),
        ]
        for (answer, expected) in cases {
            let outcome = await certifier(Relay([answer]), recorder: Recorder())
                .certifyScanned(account: "user_alice", keyA: Self.scanned)
            #expect(outcome == expected)
        }
        let relay = Relay([Self.reply(200)])
        #expect(
            await certifier(relay, recorder: Recorder(), signs: false)
                .certifyScanned(account: "user_alice", keyA: Self.scanned) == .notSigned)
        #expect(relay.sent.isEmpty)
    }

    // MARK: Settings > Device Keys (M1)

    @Test("The account's device keys come from `me`, without the removed ones")
    func listsDeviceKeys() async throws {
        let relay = Relay([
            Self.reply(
                200,
                [
                    "user": ["id": "user_alice"], "teams": [],
                    "devices": [
                        ["id": "d-1", "label": "Mac", "keyA": "ssh-ed25519 AAAAmac", "state": "trusted"],
                        ["id": "d-2", "label": "Old", "keyA": "ssh-ed25519 AAAAold", "state": "revoked"],
                        ["id": "d-3", "label": "Stranger", "keyA": "ssh-ed25519 AAAAx", "state": "pending"],
                    ],
                ])
        ])
        let rows = try await relay.client.deviceKeys().get()
        #expect(rows.map(\.id) == ["d-1", "d-3"])
        #expect(rows.map(\.isConfirmed) == [true, false])
        #expect(relay.sent.map(\.path) == ["/v1/directory/me"])
    }

    @Test("Remove revokes that device key by its id")
    func removesADeviceKey() async {
        let relay = Relay([Self.reply(200, ["ok": true, "uncertified": 0])])
        #expect((try? await relay.client.removeDeviceKey(id: "d-3").get()) != nil)
        #expect(relay.sent.map(\.path) == ["/v1/directory/devices/revoke"])
        #expect(relay.sent[0].body["device"] as? String == "d-3")
        #expect((try? await Relay([Self.reply(404)]).client.removeDeviceKey(id: "d-3").get()) == nil)
    }

    @Test("This device is the row holding its own key, comment or not, and never one the relay labels so")
    func marksThisDevice() {
        let row = DeviceKeyRow(id: "d-1", label: "Mac", keyA: "ssh-ed25519 AAAAmac", state: "trusted")
        #expect(row.isHeld(by: "ssh-ed25519 AAAAmac farcooler-MacBook"))
        #expect(!row.isHeld(by: "ssh-ed25519 AAAAphone"))
        #expect(!row.isHeld(by: nil))
        #expect(row.detail(isThisDevice: true).hasPrefix("This device. "))
        let pending = DeviceKeyRow(id: "d-2", label: "iPhone", keyA: "ssh-ed25519 AAAAphone", state: "pending")
        #expect(pending.detail(isThisDevice: false).contains("Scan its code"))
    }
}
