import AgentKit
import CryptoKit
import Foundation
import LocalAuthentication

/// The owner's "Allow cutover once", said on this Mac (ov-974).
///
/// An orchestrator asks on a card for the owner's word before it runs the
/// engine cutover; the owner's answer grants it only when it says they were
/// there. On this Mac that is a signature from this app's own key, which asks
/// for Touch ID or the login password each time it signs, sent from this app's
/// own process to its own runner (the runner also checks the app's code
/// signature, `cutover_grant::said_from`).
///
/// - **Every other answer** goes out as before, through the CLI. This one
///   can't: the runner would see the CLI, not the app.
/// - **Only this Mac's own runner.** A runner reached over ssh sees ssh, not
///   this app; there the owner grants from their phone.
/// - **Nothing answers it but the person.** Whatever reaches an answer here (a
///   button, a notification's action, an agent driving the window) ends at
///   the Touch ID prompt, which only the person at the sensor passes.
enum MacCutoverGrant {
    /// The id the runner keeps this app's key under (`step_up::LOCAL_APP`).
    static let clientID = "local:app"
    /// The host pin a statement names on the socket (`step_up::LOCAL_PIN`).
    static let hostPin = "local"
    /// What the prompt says: macOS shows it as "Far Cooler is trying to …".
    static let reason = "allow your orchestrator to run the cutover once"

    /// Answer `task` (its id) with the grant: record this app's key with the
    /// runner (the app's own, so it may replace it), then sign the answer
    /// after Touch ID or the login password and send it.
    static func answer(task: String, socket: String = RunnerCore.localSocket(), signer: StepUpSigning = MacStepUpSigner()) async throws {
        let core = RunnerCore()
        try await core.connect(socket: socket)
        let calls = RunnerCoreStepUpCalls(core: core)
        let key = try signer.publicKey ?? signer.makeKey()
        _ = try await calls.call("step_up.propose", ["publicKey": key.base64EncodedString(), "clientId": clientID])
        let step = StepUp(core: calls, signer: signer, clientId: clientID, hostPin: hostPin)
        _ = try await step.perform(
            "task.note", CutoverGrant.request(task: task), reason: reason, hasTrustedKey: true)
    }

    /// The sentence for a grant that didn't go, or nil when the person closed
    /// the prompt themselves.
    static func sentence(for error: Error) -> String? {
        if let stepUp = error as? StepUpError {
            if stepUp == .noPasscode { return "Set a login password on this Mac to allow the cutover from it." }
            if stepUp == .keyUnavailable { return "This Mac can’t sign the answer. Allow the cutover from your iPhone." }
            return StepUpWords.sentence(for: stepUp, runner: "this Mac", device: "Mac")
        }
        if let word = RunnerCoreStepUpCalls.word(error) {
            return StepUpWords.sentence(forRefusal: word, runner: "this Mac", device: "Mac")
        }
        return "The answer wasn’t sent. Try again."
    }
}

/// This app's own step-up key: a Secure Enclave P-256 key that asks for Touch
/// ID or the login password each time it signs (ov-974).
///
/// - **CryptoKit's enclave key, not a keychain item.** The app has no
///   `application-identifier`, so macOS refuses it a Secure Enclave keychain
///   key (`SecureEnclaveStepUpSigner`'s note); CryptoKit's needs none. What is
///   kept on disk is the enclave's wrapped key, which only this Mac's enclave
///   can use, and only with the person's verification.
/// - **Verified for each signature.** `.userPresence` on the key, and a fresh
///   `LAContext` with no reuse window per signature: one prompt, one answer.
/// - **No stand-in.** Nothing swaps this for a key that signs without a
///   person: not a launch argument, an environment variable or a default.
final class MacStepUpSigner: StepUpSigning, @unchecked Sendable {
    private let file: URL

    init(file: URL = MacStepUpSigner.defaultFile) {
        self.file = file
    }

    static var defaultFile: URL {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return support.appendingPathComponent(Bundle.main.bundleIdentifier ?? "com.farcooler.FarCooler")
            .appendingPathComponent("step-up-key.enclave")
    }

    var publicKey: Data? {
        guard let blob = try? Data(contentsOf: file),
            let key = try? SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob)
        else { return nil }
        return key.publicKey.x963Representation
    }

    func makeKey() throws -> Data {
        if let existing = publicKey { return existing }
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
        try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try key.dataRepresentation.write(to: file, options: .atomic)
        try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        return key.publicKey.x963Representation
    }

    func sign(_ statement: Data, reason: String) async throws -> Data {
        guard let blob = try? Data(contentsOf: file) else { throw StepUpError.keyUnavailable }
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
