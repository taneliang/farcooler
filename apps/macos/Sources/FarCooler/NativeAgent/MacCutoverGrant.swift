import AgentKit
import CryptoKit
import Foundation
import LocalAuthentication

/// The owner's word on this Mac, signed (ov-974): "Allow cutover once", and
/// the approval that vouches for a phone's key so it may say it too.
///
/// An orchestrator asks on a card for the owner's word before it runs the
/// engine cutover; the owner's answer grants it only when it says they were
/// there. On this Mac that is a signature from a key this app makes for the
/// one signature, which asks for Touch ID or the login password, sent from
/// this app's own process to its own runner (the runner also checks the app's
/// code signature, `cutover_grant::said_from`).
///
/// - **Every other answer** goes out as before, through the CLI. These can't:
///   the runner would see the CLI, not the app.
/// - **Only this Mac's own runner.** A runner reached over ssh sees ssh, not
///   this app; there the owner grants from their phone.
/// - **Nothing answers it but the person.** Whatever reaches an answer here (a
///   button, a notification's action, an agent driving the window) ends at
///   the Touch ID prompt, which only the person at the sensor passes.
enum MacOwnerStepUp {
    /// The id the runner keeps this app's key under (`step_up::LOCAL_APP`).
    static let clientID = "local:app"
    /// The host pin a statement names on the socket (`step_up::LOCAL_PIN`).
    static let hostPin = "local"

    /// `method` with `args`, signed after Touch ID or the login password:
    /// a fresh key for this one signature (B2 of the second review), recorded
    /// with the runner (the app's own, so it may replace it), then the
    /// signature. `reason` is what the prompt says after "Far Cooler is
    /// trying to".
    @discardableResult
    static func perform(
        _ method: String, _ args: [String: Any], reason: String, socket: String = RunnerCore.localSocket(),
        signer: StepUpSigning = MacStepUpSigner()
    ) async throws -> Data {
        let core = RunnerCore()
        try await core.connect(socket: socket)
        let calls = RunnerCoreStepUpCalls(core: core)
        let key = try signer.makeKey()
        _ = try await calls.call("step_up.propose", ["publicKey": key.base64EncodedString(), "clientId": clientID])
        let step = StepUp(core: calls, signer: signer, clientId: clientID, hostPin: hostPin)
        return try await step.perform(method, args, reason: reason, hasTrustedKey: true)
    }

    /// The sentence for a signed action that didn't go, or nil when the
    /// person closed the prompt themselves.
    static func sentence(for error: Error, doing: String) -> String? {
        if let stepUp = error as? StepUpError {
            if stepUp == .noPasscode { return "Set a login password on this Mac to \(doing) from it." }
            if stepUp == .keyUnavailable { return "This Mac can’t sign this. Use your iPhone to \(doing)." }
            return StepUpWords.sentence(for: stepUp, runner: "this Mac", device: "Mac")
        }
        if let word = RunnerCoreStepUpCalls.word(error) {
            return StepUpWords.sentence(forRefusal: word, runner: "this Mac", device: "Mac")
        }
        return "That wasn’t sent. Try again."
    }
}

/// "Allow cutover once", said on this Mac.
enum MacCutoverGrant {
    /// Answer `task` (its id, `card` its key) with the grant, signed.
    static func answer(task: String, card: String, socket: String = RunnerCore.localSocket(), signer: StepUpSigning = MacStepUpSigner()) async throws {
        try await MacOwnerStepUp.perform(
            "task.note", CutoverGrant.request(task: task), reason: reason(card: card), socket: socket, signer: signer)
    }

    /// What the prompt says, naming the runner and the card (M3).
    static func reason(card: String) -> String {
        "allow your orchestrator on this Mac to run the cutover once, as \(card) asks"
    }

    static func sentence(for error: Error) -> String? {
        MacOwnerStepUp.sentence(for: error, doing: "allow the cutover")
    }
}

/// A key this app makes for one signature: a Secure Enclave P-256 key that
/// asks for Touch ID or the login password when it signs (ov-974).
///
/// - **Made here, for each signature, kept nowhere** (B2 of the second
///   review). The key is made in this process and held in memory; nothing is
///   read from disk, so no file another process writes can stand in a key
///   that signs without the person.
/// - **CryptoKit's enclave key, not a keychain item.** The app has no
///   `application-identifier`, so macOS refuses it a Secure Enclave keychain
///   key (`SecureEnclaveStepUpSigner`'s note); CryptoKit's needs none.
/// - **Verified for the signature.** `.userPresence` on the key, and a fresh
///   `LAContext` with no reuse window: one prompt, one signature.
/// - **No stand-in.** Nothing swaps this for a key that signs without a
///   person: not a launch argument, an environment variable or a default.
final class MacStepUpSigner: StepUpSigning, @unchecked Sendable {
    /// The enclave's wrapped key, which only this Mac's enclave can use, and
    /// only with the person's verification. In memory alone.
    private var blob: Data?

    init() {}

    var publicKey: Data? {
        guard let blob, let key = try? SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob) else { return nil }
        return key.publicKey.x963Representation
    }

    /// A new key, every time: never one that was made before.
    func makeKey() throws -> Data {
        guard SecureEnclave.isAvailable else { throw StepUpError.keyUnavailable }
        var failure: Unmanaged<CFError>?
        guard
            let control = SecAccessControlCreateWithFlags(
                nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, [.privateKeyUsage, .userPresence], &failure)
        else { throw StepUpError.noPasscode }
        let key: SecureEnclave.P256.Signing.PrivateKey
        do { key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: control) } catch {
            throw StepUpError.keyUnavailable
        }
        blob = key.dataRepresentation
        return key.publicKey.x963Representation
    }

    func sign(_ statement: Data, reason: String) async throws -> Data {
        guard let blob else { throw StepUpError.keyUnavailable }
        return try await Task.detached(priority: .userInitiated) {
            let context = LAContext()
            context.touchIDAuthenticationAllowableReuseDuration = 0
            context.localizedReason = reason
            do {
                let key = try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob, authenticationContext: context)
                // ECDSA over SHA-256 of the statement, as the runner verifies it.
                return try key.signature(for: statement).derRepresentation
            } catch {
                let code = (error as NSError).code
                // errSecUserCanceled, errSecAuthFailed, and LAError's cancel codes.
                if code == -128 || code == -2 || code == -4 || code == -9 { throw StepUpError.cancelled }
                throw StepUpError.keyUnavailable
            }
        }.value
    }
}
