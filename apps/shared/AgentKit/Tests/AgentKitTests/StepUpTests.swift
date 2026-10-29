import CryptoKit
import Foundation
import Testing

@testable import AgentKit

/// Step-up from the app's side (ov-856, slice 4): the order of the calls, one
/// prompt per action, the keyless transition, and the words. The key is a
/// CryptoKit P-256 key standing where the Secure Enclave would, signing the
/// same way (DER over SHA-256 of the statement).
struct StepUpTests {
    /// A core that answers `step_up.*` as the client core does and records
    /// every call.
    final class Core: StepUpCalling, @unchecked Sendable {
        var calls: [(method: String, args: [String: Any])] = []
        var refuseFirstPlainWith: String?
        let statement = Data("farcooler-step-up-v1 statement".utf8)

        struct Refused: Error { let word: String }

        func call(_ method: String, _ args: [String: Any]) async throws -> Data {
            calls.append((method, args))
            switch method {
            case "step_up.begin":
                return try JSONSerialization.data(withJSONObject: ["token": "t-1", "statement": statement.base64EncodedString()])
            case "step_up.finish": return Data(#"{"clients":[]}"#.utf8)
            case "step_up.propose": return Data(#"{"clientId":"phone-o","state":"proposed","code":"418207"}"#.utf8)
            default:
                if let word = refuseFirstPlainWith {
                    refuseFirstPlainWith = nil
                    throw Refused(word: word)
                }
                return Data(#"{"clients":["plain"]}"#.utf8)
            }
        }

        func refusalWord(_ error: Error) -> String? { (error as? Refused)?.word }

        var methods: [String] { calls.map(\.method) }
    }

    final class Signer: StepUpSigning, @unchecked Sendable {
        private var key: P256.Signing.PrivateKey?
        var prompts: [String] = []
        var cancels = false
        var made = 0

        init(hasKey: Bool = true) { key = hasKey ? P256.Signing.PrivateKey() : nil }

        var publicKey: Data? { key?.publicKey.x963Representation }

        func makeKey() throws -> Data {
            made += 1
            key = P256.Signing.PrivateKey()
            return key!.publicKey.x963Representation
        }

        func sign(_ statement: Data, reason: String) async throws -> Data {
            prompts.append(reason)
            if cancels { throw StepUpError.cancelled }
            return try key!.signature(for: statement).derRepresentation
        }
    }

    func step(_ core: Core, _ signer: Signer) -> StepUp {
        StepUp(core: core, signer: signer, clientId: "phone-o", hostPin: "SHA256:host")
    }

    @Test("A device with a trusted key signs: begin, one prompt, finish with that signature")
    func signed() async throws {
        let core = Core()
        let signer = Signer()
        let out = try await step(core, signer).perform(
            "client.revoke", ["clientId": "pad-x"], reason: "Remove iPad from studio.", hasTrustedKey: true)
        #expect(String(decoding: out, as: UTF8.self) == #"{"clients":[]}"#)
        #expect(core.methods == ["step_up.begin", "step_up.finish"], "the plain call is never made")
        let begin = core.calls[0].args
        #expect(begin["method"] as? String == "client.revoke")
        #expect(begin["clientId"] as? String == "phone-o")
        #expect(begin["hostPin"] as? String == "SHA256:host")
        #expect((begin["args"] as? [String: Any])?["clientId"] as? String == "pad-x")
        #expect(signer.prompts == ["Remove iPad from studio."], "one prompt, saying what it's for")
        let finish = core.calls[1].args
        #expect(finish["token"] as? String == "t-1")
        let signature = try #require(Data(base64Encoded: finish["signature"] as? String ?? ""))
        let key = try P256.Signing.PublicKey(x963Representation: #require(signer.publicKey))
        #expect(key.isValidSignature(try P256.Signing.ECDSASignature(derRepresentation: signature), for: core.statement),
            "the DER signature over the statement the core built")
    }

    @Test("Without a trusted key the old way stands, and nobody is asked for a face")
    func keyless() async throws {
        let core = Core()
        let signer = Signer()
        let out = try await step(core, signer).perform("client.revoke", ["clientId": "x"], reason: "r", hasTrustedKey: false)
        #expect(String(decoding: out, as: UTF8.self) == #"{"clients":["plain"]}"#)
        #expect(core.methods == ["client.revoke"] && signer.prompts.isEmpty)
    }

    @Test("A runner that asks for a signature anyway gets one, once, and only for that word")
    func asked() async throws {
        let core = Core()
        core.refuseFirstPlainWith = "step_up_needed"
        let signer = Signer()
        _ = try await step(core, signer).perform("client.revoke", ["clientId": "x"], reason: "r", hasTrustedKey: false)
        #expect(core.methods == ["client.revoke", "step_up.begin", "step_up.finish"])

        let other = Core()
        other.refuseFirstPlainWith = "scope-denied"
        await #expect(throws: Core.Refused.self) {
            _ = try await step(other, Signer()).perform("client.revoke", ["clientId": "x"], reason: "r", hasTrustedKey: false)
        }
        #expect(other.methods == ["client.revoke"], "any other refusal is the caller's to show")

        let keyless = Core()
        keyless.refuseFirstPlainWith = "step_up_needed"
        await #expect(throws: Core.Refused.self) {
            _ = try await step(keyless, Signer(hasKey: false)).perform("client.revoke", [:], reason: "r", hasTrustedKey: false)
        }
    }

    @Test("Closing the prompt sends nothing")
    func cancelled() async throws {
        let core = Core()
        let signer = Signer()
        signer.cancels = true
        await #expect(throws: StepUpError.cancelled) {
            _ = try await step(core, signer).perform("client.revoke", [:], reason: "r", hasTrustedKey: true)
        }
        #expect(core.methods == ["step_up.begin"], "the action is never finished")
    }

    @Test("Registering makes the key once and shows the digits the runner gave")
    func register() async throws {
        let core = Core()
        let fresh = Signer(hasKey: false)
        let code = try await step(core, fresh).register()
        #expect(code == "418207" && fresh.made == 1)
        let sent = try #require(core.calls.first?.args["publicKey"] as? String)
        #expect(Data(base64Encoded: sent) == fresh.publicKey, "the key sent is the key held")
        #expect(Data(base64Encoded: sent)?.count == 65)

        let held = Signer()
        _ = try await step(Core(), held).register()
        #expect(held.made == 0, "an existing key is offered, not replaced")
    }

    @Test("A prompt says what it is for, as the design words it")
    func reasons() {
        #expect(StepUpReason.remove(person: "Cosmo", runner: "studio") == "Remove Cosmo from studio.")
        #expect(StepUpReason.add(device: "Cosmo’s iPhone", runner: "studio", access: .member) == "Add Cosmo’s iPhone to studio as a Member.")
        #expect(StepUpReason.add(device: "x", runner: "studio", access: .admin) == "Add x to studio as an Admin.")
        #expect(StepUpReason.removeDevice("E-Liang’s iPad", alsoRemoving: 2, runner: "studio") == "Remove E-Liang’s iPad and the 2 devices it added.")
        #expect(StepUpReason.removeDevice("iPad", alsoRemoving: 1, runner: "studio") == "Remove iPad and the device it added.")
        #expect(StepUpReason.removeDevice("iPad", alsoRemoving: 0, runner: "studio") == "Remove iPad from studio.")
    }

    @Test("A refusal is a sentence, and closing the prompt is silence")
    func refusals() {
        let say = { (word: String) in StepUpWords.sentence(for: .refused(word), runner: "studio", device: "iPhone") }
        #expect(say("step_up_needed") == "Allow admin actions on this iPhone from studio first.")
        #expect(say("key_untrusted") == "Allow admin actions on this iPhone from studio first.")
        #expect(say("step_up") == "Update Far Cooler on studio to change access here.")
        #expect(say("challenge_expired") == "That took too long. Try again.")
        #expect(say("unheard_of") == "Couldn’t do that. Try again.")
        #expect(StepUpWords.sentence(for: .cancelled, runner: "studio", device: "iPhone") == nil)
        #expect(!say("host_pin_mismatch")!.contains("host_pin"), "no runner word reaches the screen")
    }
}

/// The app lock (§3a): five minutes away, then a face before terminals and
/// admin screens.
struct AppLockTests {
    let t0 = Date(timeIntervalSince1970: 1_000_000)

    @Test("Away for five minutes locks; four minutes fifty-nine doesn't")
    func idle() {
        var near = AppLock()
        near.didLeave(at: t0)
        near.didReturn(at: t0.addingTimeInterval(AppLock.idleLimit - 1))
        #expect(!near.isLocked)
        var far = AppLock()
        far.didLeave(at: t0)
        far.didReturn(at: t0.addingTimeInterval(AppLock.idleLimit))
        #expect(far.isLocked)
        #expect(AppLock.idleLimit == 300)
    }

    @Test("A launch is locked, and an unlock lasts until the next long absence")
    func launch() {
        var lock = AppLock.atLaunch()
        #expect(lock.isLocked)
        lock.unlock()
        lock.didLeave(at: t0)
        lock.didReturn(at: t0.addingTimeInterval(60))
        #expect(!lock.isLocked)
        lock.didLeave(at: t0)
        lock.didReturn(at: t0.addingTimeInterval(600))
        #expect(lock.isLocked)
    }

    @Test("It covers terminals and admin screens, and nothing else")
    func covers() {
        let lock = AppLock(isOn: true, isLocked: true)
        #expect(lock.covers(.terminal) && lock.covers(.admin) && !lock.covers(.other))
        #expect(!AppLock(isOn: false, isLocked: true).covers(.terminal), "turned off")
        #expect(!AppLock().covers(.terminal), "unlocked")
    }

    @Test("A tap or a link waits while the lock covers terminals, and opens when it doesn't")
    func entriesWaitForTheLock() {
        var lock = AppLock.atLaunch()
        #expect(!lock.admits(.terminal) && !lock.admits(.admin), "a notification or link can't open a terminal under the lock")
        lock.unlock()
        #expect(lock.admits(.terminal))
        #expect(AppLock(isOn: false, isLocked: true).admits(.terminal), "off means off")
    }

    @Test("Turning the lock off needs a face first; turning it on never does")
    func turningOffAsks() {
        #expect(AppLock.setting(wanting: false, verified: false), "unverified: it stays on")
        #expect(!AppLock.setting(wanting: false, verified: true))
        #expect(AppLock.setting(wanting: true, verified: false))
    }

    @Test("A resume with no record of leaving is locked, never open")
    func noRecord() {
        var lock = AppLock()
        lock.didReturn(at: t0)
        #expect(lock.isLocked)
    }
}
