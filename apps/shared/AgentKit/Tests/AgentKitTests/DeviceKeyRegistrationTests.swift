import Foundation
import Testing

@testable import AgentKit

/// A device registering its Key A on sign-in (ov-549): the two calls the app
/// makes, what it signs, and what it does with each answer.
@MainActor
struct DeviceKeyRegistrationTests {
    static let challenge = String(repeating: "ab", count: 32)
    static let proof = DeviceKeyProof(keyA: "ssh-ed25519 AAAAkey", proofA: "-----BEGIN SSH SIGNATURE-----\nx\n-----END SSH SIGNATURE-----\n")

    /// A relay that answers from a script and records what it was sent.
    @MainActor final class Relay {
        var challenge: Result<DirectoryReply, AccountError> = .success(
            DirectoryReply(status: 200, body: ["challenge": DeviceKeyRegistrationTests.challenge, "expiresAt": 1]))
        var register: Result<DirectoryReply, AccountError> = .success(
            DirectoryReply(status: 200, body: ["device": "d-1", "state": "trusted"]))
        private(set) var sent: [(path: String, body: [String: Any])] = []

        var client: DirectoryClient {
            DirectoryClient { path, body in
                self.sent.append((path, body))
                switch path {
                case "/v1/directory/devices/challenge": return self.challenge
                case "/v1/directory/devices/register": return self.register
                default: return .failure(.endpointMissing)
                }
            }
        }
    }

    /// What the signer was asked to sign, by whom.
    @MainActor final class Signer {
        private(set) var asked: [(account: String, challenge: String)] = []
        var answer: DeviceKeyProof? = DeviceKeyRegistrationTests.proof
        func sign(_ account: String, _ challenge: String) -> DeviceKeyProof? {
            asked.append((account, challenge))
            return answer
        }
    }

    func registration(_ signer: Signer) -> DeviceKeyRegistration {
        let registration = DeviceKeyRegistration()
        registration.label = { "Alice’s iPhone" }
        registration.sign = { signer.sign($0, $1) }
        return registration
    }

    @Test("Sign-in asks for a challenge, signs it for this device's own account, and registers the proof")
    func signInRegistersTheKey() async throws {
        let relay = Relay()
        let signer = Signer()
        let registration = registration(signer)
        let outcome = await registration.registerIfNeeded(account: "user_alice", directory: relay.client)
        #expect(outcome == .success(.trusted))
        #expect(registration.state == .trusted)
        #expect(relay.sent.map(\.path) == ["/v1/directory/devices/challenge", "/v1/directory/devices/register"])
        #expect(signer.asked.map(\.account) == ["user_alice"])
        #expect(signer.asked.map(\.challenge) == [Self.challenge])
        let body = relay.sent[1].body
        #expect(body["label"] as? String == "Alice’s iPhone")
        #expect(body["challenge"] as? String == Self.challenge)
        #expect(body["keyA"] as? String == Self.proof.keyA)
        #expect(body["proofA"] as? String == Self.proof.proofA)
        #expect(Set(body.keys) == ["label", "challenge", "keyA", "proofA"])
    }

    @Test("Once per account per launch, and again for the next account signed in")
    func oncePerAccount() async {
        let relay = Relay()
        let signer = Signer()
        let registration = registration(signer)
        await registration.registerIfNeeded(account: "user_alice", directory: relay.client)
        #expect(await registration.registerIfNeeded(account: "user_alice", directory: relay.client) == nil)
        await registration.registerIfNeeded(account: "user_bob", directory: relay.client)
        #expect(signer.asked.map(\.account) == ["user_alice", "user_bob"])
    }

    @Test("A sign-in during the launch's registration is queued and run after it, not dropped")
    func aSignInDuringALaunchRegistrationIsQueued() async {
        let signer = Signer()
        let registration = registration(signer)
        // The launch's challenge waits until released, as a slow relay would.
        var release: CheckedContinuation<Void, Never>?
        var held = true
        let slow = DirectoryClient { path, _ in
            guard path == "/v1/directory/devices/challenge" else {
                return .success(DirectoryReply(status: 200, body: ["device": "d", "state": "trusted"]))
            }
            if held {
                held = false
                await withCheckedContinuation { release = $0 }
            }
            return .success(DirectoryReply(status: 200, body: ["challenge": Self.challenge]))
        }
        let launch = Task { await registration.registerIfNeeded(account: "user_alice", directory: slow) }
        while release == nil { await Task.yield() }
        // Signed out and in as Bob while Alice's is in flight.
        #expect(await registration.registerIfNeeded(account: "user_bob", directory: slow) == nil)
        release?.resume()
        #expect(await launch.value == .success(.trusted))
        #expect(signer.asked.map(\.account) == ["user_alice", "user_bob"])
        // Bob is done; asking again does nothing.
        #expect(await registration.registerIfNeeded(account: "user_bob", directory: slow) == nil)
    }

    @Test("Signed out, nothing is asked")
    func signedOutAsksNothing() async {
        let relay = Relay()
        #expect(await registration(Signer()).registerIfNeeded(account: "", directory: relay.client) == nil)
        #expect(relay.sent.isEmpty)
    }

    @Test("A signer that refuses sends nothing more, and a failure is tried again next time")
    func aRefusalSendsNothing() async {
        let relay = Relay()
        let signer = Signer()
        signer.answer = nil
        let registration = registration(signer)
        #expect(await registration.registerIfNeeded(account: "user_alice", directory: relay.client) == .failure(.proofRefused))
        #expect(relay.sent.map(\.path) == ["/v1/directory/devices/challenge"])
        signer.answer = Self.proof
        #expect(await registration.registerIfNeeded(account: "user_alice", directory: relay.client) == .success(.trusted))
    }

    @Test("Each answer the relay can give comes to a state or a failure, never a guess")
    func answers() async {
        let cases: [(Result<DirectoryReply, AccountError>, Result<DirectoryReply, AccountError>?, Result<DeviceKeyState, DirectoryFailure>)] = [
            (.success(DirectoryReply(status: 404, body: [:])), nil, .failure(.account(.endpointMissing))),
            (.success(DirectoryReply(status: 200, body: [:])), nil, .failure(.account(.malformedResponse))),
            (.failure(.unauthorized), nil, .failure(.account(.unauthorized))),
            (.success(DirectoryReply(status: 200, body: ["challenge": Self.challenge])),
             .success(DirectoryReply(status: 200, body: ["device": "d", "state": "pending"])), .success(.pending)),
            (.success(DirectoryReply(status: 200, body: ["challenge": Self.challenge])),
             .success(DirectoryReply(status: 403, body: ["error": "challenge"])), .failure(.proofRefused)),
            (.success(DirectoryReply(status: 200, body: ["challenge": Self.challenge])),
             .success(DirectoryReply(status: 409, body: ["error": "claimed"])), .failure(.alreadyRegistered)),
            (.success(DirectoryReply(status: 200, body: ["challenge": Self.challenge])),
             .success(DirectoryReply(status: 200, body: ["state": "owned"])), .failure(.account(.malformedResponse))),
        ]
        for (challenge, register, expected) in cases {
            let relay = Relay()
            relay.challenge = challenge
            if let register { relay.register = register }
            let outcome = await registration(Signer()).registerIfNeeded(account: "user_alice", directory: relay.client)
            #expect(outcome == expected)
        }
    }

    @Test("The core's answer reads as a proof, and its refusal as none")
    func readsTheCoresAnswer() {
        let answer = #"{"keyA":"ssh-ed25519 AAAAkey","proofA":"-----BEGIN SSH SIGNATURE-----"}"#
        #expect(DeviceKeyProof(answer: Data(answer.utf8)) == DeviceKeyProof(keyA: "ssh-ed25519 AAAAkey", proofA: "-----BEGIN SSH SIGNATURE-----"))
        for refusal in [#"{"error":"challenge"}"#, #"{"keyA":"ssh-rsa AAAA","proofA":"x"}"#, "", "[]"] {
            #expect(DeviceKeyProof(answer: Data(refusal.utf8)) == nil)
        }
    }
}
