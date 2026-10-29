import Foundation

/// Step-up on an admin action (ov-856, §3 of the design): one Face ID or
/// biometric prompt per action, with the passcode as a fallback, signing the
/// runner's challenge for that exact action.
///
/// **What this file owns:** the order of the calls (the client core builds
/// the bytes and the statement; the device key signs it; the core sends it),
/// the words a prompt and a refusal use, and nothing else. The key lives in
/// the app (`SecureEnclaveStepUpSigner`, Android's Keystore); the bytes that
/// are signed are the core's (`crates/client/src/session/step_up.rs`), the
/// same on every platform.

/// What a step-up needs of the client core: a call by wire name and JSON
/// arguments, and the stable word a refusal carries.
public protocol StepUpCalling {
    func call(_ method: String, _ args: [String: Any]) async throws -> Data
    /// The runner's `what` for a refusal (`step_up_needed`), nil for any
    /// other failure. Read from the error rather than its prose.
    func refusalWord(_ error: Error) -> String?
}

/// A device key that signs with the person's own verification.
public protocol StepUpSigning {
    /// The 65-byte uncompressed P-256 point, nil before a key exists.
    var publicKey: Data? { get }
    /// Make the key if this device has none, answering its public point.
    func makeKey() throws -> Data
    /// ECDSA over SHA-256 of `statement`, DER encoded, after the person
    /// verifies. `reason` is what the prompt says.
    func sign(_ statement: Data, reason: String) async throws -> Data
}

public enum StepUpError: Error, Equatable {
    /// The person closed the prompt. Nothing to say.
    case cancelled
    /// This device has no passcode, so it can't hold a key that needs one.
    case noPasscode
    /// The device can't make or use the key.
    case keyUnavailable
    /// The runner refused, by its word.
    case refused(String)
}

/// One admin action, signed.
public struct StepUp {
    public let core: StepUpCalling
    public let signer: StepUpSigning
    /// This device's own id on the runner (what its line names).
    public let clientId: String
    /// The runner's host key as this device pinned it, never as the runner said.
    public let hostPin: String

    public init(core: StepUpCalling, signer: StepUpSigning, clientId: String, hostPin: String) {
        self.core = core
        self.signer = signer
        self.clientId = clientId
        self.hostPin = hostPin
    }

    /// `method` with `args` (the ones the plain call takes), asking for
    /// the person's verification when this device holds a trusted key.
    /// A device without one keeps its old way until it registers (the
    /// transition); and a runner that asks for a signature anyway
    /// (`step_up_needed`) gets one if there's a key to sign with.
    public func perform(_ method: String, _ args: [String: Any], reason: String, hasTrustedKey: Bool) async throws -> Data {
        if hasTrustedKey { return try await signed(method, args, reason: reason) }
        do {
            return try await core.call(method, args)
        } catch {
            guard core.refusalWord(error) == "step_up_needed", signer.publicKey != nil else { throw error }
            return try await signed(method, args, reason: reason)
        }
    }

    private func signed(_ method: String, _ args: [String: Any], reason: String) async throws -> Data {
        let prepared = try await core.call(
            "step_up.begin", ["method": method, "args": args, "clientId": clientId, "hostPin": hostPin])
        guard let object = try? JSONSerialization.jsonObject(with: prepared) as? [String: Any],
            let token = object["token"] as? String, let encoded = object["statement"] as? String,
            let statement = Data(base64Encoded: encoded)
        else { throw StepUpError.keyUnavailable }
        let signature = try await signer.sign(statement, reason: reason)
        return try await core.call("step_up.finish", ["token": token, "signature": signature.base64EncodedString()])
    }

    /// Offer this device's key to the runner for approval: makes the key
    /// if need be, and answers the six digits to show beside "Allow Admin
    /// Actions", which another device of the same person checks.
    public func register() async throws -> String {
        let key = try signer.publicKey ?? signer.makeKey()
        let answer = try await core.call("step_up.propose", ["publicKey": key.base64EncodedString()])
        guard let object = try? JSONSerialization.jsonObject(with: answer) as? [String: Any],
            let code = object["code"] as? String, code.count == 6
        else { throw StepUpError.keyUnavailable }
        return code
    }
}

/// What a prompt says, per action (§8 of the design).
public enum StepUpReason {
    public static func remove(person: String, runner: String) -> String {
        "Remove \(person) from \(runner)."
    }

    public static func removeDevice(_ device: String, alsoRemoving added: Int, runner: String) -> String {
        switch added {
        case 0: "Remove \(device) from \(runner)."
        case 1: "Remove \(device) and the device it added."
        default: "Remove \(device) and the \(added) devices it added."
        }
    }

    public static func add(device: String, runner: String, access: PersonAccess) -> String {
        "Add \(device) to \(runner) as \(article(access.title)) \(access.title)."
    }

    public static func approve(device: String) -> String {
        "Allow \(device) to approve changes to who has access."
    }

    public static func limit(_ device: String, runner: String) -> String {
        "Limit \(device) on \(runner) to Viewer."
    }

    public static func setAccess(_ person: String, to access: PersonAccess, runner: String) -> String {
        "Make \(person) \(article(access.title)) \(access.title) on \(runner)."
    }

    private static func article(_ word: String) -> String { "AEIOU".contains(word.first ?? " ") ? "an" : "a" }
}

/// What a refusal says, in sentences with no runner error in them.
public enum StepUpWords {
    /// What `error` means to the person, or nil when they closed the
    /// prompt themselves and nothing should be said.
    public static func sentence(for error: StepUpError, runner: String, device: String) -> String? {
        switch error {
        case .cancelled: nil
        case .noPasscode: "Set a passcode on this \(device) to allow admin actions."
        case .keyUnavailable: "This \(device) can’t keep an admin key. Use another device."
        case .refused(let word): sentence(forRefusal: word, runner: runner, device: device)
        }
    }

    public static func sentence(forRefusal word: String, runner: String, device: String) -> String {
        switch word {
        case "step_up_needed", "key_untrusted", "key_untrusted_here":
            "Allow admin actions on this \(device) from \(runner) first."
        case "step_up_unneeded", "step_up_unavailable", "step_up":
            "Update Far Cooler on \(runner) to change access here."
        case "challenge_expired":
            "That took too long. Try again."
        case "step_up_failed", "host_pin_mismatch", "step_up_replayed":
            "Couldn’t confirm it was you. Try again."
        case "defensive_only", "not_admin", "not_yours", "socket_only", "person_only":
            "You can’t change that from this \(device)."
        case "add_limit":
            "You can add five devices a day from this \(device). Try again tomorrow."
        case "admin_is_owners":
            "Only the Owner can make someone an Admin."
        case "access_exists":
            "They already have a different access on \(runner). Change it from People."
        case "handle_taken", "device_taken":
            "That person or device is already on \(runner), under another name."
        case "scope_access":
            "A Viewer’s device can only look. Choose Viewer, or add it as a Member."
        case "wrong_code":
            "That code doesn’t match. Check the other device and try again."
        case "no_approver":
            "Open Far Cooler on your iPhone and allow admin actions there first."
        case "approval_denied":
            "Your iPhone didn’t approve it."
        case "approval_expired":
            "Your iPhone didn’t approve it in time. Try again."
        case "proposal_expired":
            "That request timed out. Ask again on the other device."
        default:
            "Couldn’t do that. Try again."
        }
    }
}
