import Foundation
import os

/// How a certification in the same-account ceremony ended (ov-550).
public enum DeviceCertificationOutcome: Equatable, Sendable {
    /// The new device is confirmed as this account's.
    case certified
    /// It already was, by this device or another.
    case alreadyConfirmed
    /// The new device never registered its key with the account, even after
    /// waiting: it isn't signed in, or it couldn't reach the relay.
    case notRegistered
    /// The relay refused the certificate. Most often this device isn't
    /// confirmed itself, so it vouches for nothing.
    case refused
    /// Nothing was sent: no account signed in here, a code from another
    /// account, or the core wouldn't sign.
    case notSigned
    /// The relay couldn't be asked, or answered something else.
    case failed
}

extension DirectoryClient {
    /// Certify the device whose Key A this one scanned, with `certificate`,
    /// this device's signature over it (`farcooler_client_device_certificate`).
    ///
    /// The device is named by `keyA`, exactly as the code carried it: the
    /// relay finds it among the signed-in account's devices.
    public func certifyDevice(keyA: String, certificate: String) async -> DeviceCertificationOutcome {
        switch await post("/v1/directory/devices/certify", ["keyA": keyA, "certificate": certificate]) {
        case .failure:
            return .failed
        case .success(let reply):
            switch (reply.status, reply.body["error"] as? String) {
            case (200, _): return .certified
            case (404, _): return .notRegistered
            case (403, _): return .refused
            case (409, "state"):
                let state = reply.body["state"] as? String
                return state == "trusted" || state == "certified" ? .alreadyConfirmed : .refused
            default: return .failed
            }
        }
    }
}

/// Certifying the device scanned in the same-account ceremony (ov-550,
/// ov-480 §3): the trusted device signs, with its own Key A, a certificate
/// for the Key A in the code it read, and posts it.
///
/// **The key comes from the scanned code, never from the relay.** A directory
/// that could choose what a device certifies could add "your new laptop"
/// itself. And the account is this device's own record of who is signed in,
/// and only when the code named the same one: the ceremony's core has already
/// refused a code for another account (`wrong_account`), and this checks again
/// before signing, so a teammate's code is never certified.
///
/// The new device registers its key when it signs in, which can land a
/// moment after its code goes up, so a 404 is retried a few times before
/// giving up. Nothing here reaches a screen; the log names outcomes only.
@MainActor
public final class DeviceCertification {
    public static let shared = DeviceCertification()

    /// Sign a certificate, as this device for `account`, for the scanned
    /// `keyA`: the app's, since the key is the app's. Nil when it can't.
    public var sign: (_ account: String, _ keyA: String) -> String? = { _, _ in nil }
    /// Who is signed in here, and the relay to ask. `Account.shared` in the
    /// app; a test answers both.
    public var signedIn: () -> (account: String, directory: DirectoryClient)? = {
        let account = Account.shared
        return account.isSignedIn ? (account.userId, account.directoryClient) : nil
    }
    /// How long to wait before each retry of a device not registered yet.
    public var waits: [Duration] = [.seconds(2), .seconds(5), .seconds(15), .seconds(30), .seconds(60)]
    public var pause: (Duration) async -> Void = { try? await Task.sleep(for: $0) }

    private static let log = Logger(subsystem: "com.farcooler.agentkit", category: "device-keys")

    public init() {}

    /// Certify the device whose code named `account` and carried `keyA`.
    @discardableResult
    public func certifyScanned(account: String, keyA: String) async -> DeviceCertificationOutcome {
        let outcome = await run(account: account, keyA: keyA)
        Self.log.notice("device certification: \(String(describing: outcome), privacy: .public)")
        return outcome
    }

    private func run(account: String, keyA: String) async -> DeviceCertificationOutcome {
        guard !account.isEmpty, let here = signedIn(), here.account == account,
            let certificate = sign(account, keyA)
        else { return .notSigned }
        var outcome = await here.directory.certifyDevice(keyA: keyA, certificate: certificate)
        for wait in waits where outcome == .notRegistered {
            await pause(wait)
            // Signed out, or someone else signed in, while waiting.
            guard let now = signedIn(), now.account == account else { return .notSigned }
            outcome = await now.directory.certifyDevice(keyA: keyA, certificate: certificate)
        }
        return outcome
    }
}
