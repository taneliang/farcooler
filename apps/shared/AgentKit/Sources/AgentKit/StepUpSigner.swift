#if os(iOS)
import Foundation
import LocalAuthentication
import Security

/// A Secure Enclave P-256 key that asks the person each time it signs
/// (ov-856, §3 of the design).
///
/// - **Device only.** `kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly`: the
///   key never leaves this device and exists only while a passcode is set.
/// - **Verified for each signature.** `.userPresence` is Face ID or Touch ID
///   with the passcode as the fallback, on the key itself; and each signature
///   gets a fresh `LAContext` with no reuse window, so one prompt is one
///   action and nothing is remembered between two.
/// - **iPhone and iPad only (ov-858).** The Mac app is signed ad hoc, with no
///   `application-identifier` or provisioning profile, so macOS refuses it a
///   Secure Enclave keychain key. A Mac that isn't the runner asks the
///   person's iPhone to approve instead (`PhoneApproval`); this type isn't
///   compiled for macOS, so no Mac path can reach a key it can't have.
/// - **The Simulator has no enclave.** There the key is an ordinary keychain
///   key with no prompt, so the flows can be driven; a device never takes
///   that path.
public final class SecureEnclaveStepUpSigner: StepUpSigning, @unchecked Sendable {
    private let tag: Data

    public init(tag: String = "sg.elt.farcooler.step-up") {
        self.tag = Data(tag.utf8)
    }

    public var publicKey: Data? {
        guard let key = privateKey(context: nil), let point = SecKeyCopyPublicKey(key) else { return nil }
        return SecKeyCopyExternalRepresentation(point, nil) as Data?
    }

    public func makeKey() throws -> Data {
        if let existing = publicKey { return existing }
        var attributes: [String: Any] = [
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecAttrKeySizeInBits as String: 256,
        ]
        var privateAttributes: [String: Any] = [
            kSecAttrIsPermanent as String: true,
            kSecAttrApplicationTag as String: tag,
        ]
        #if !targetEnvironment(simulator)
        var failure: Unmanaged<CFError>?
        guard
            let control = SecAccessControlCreateWithFlags(
                nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, [.privateKeyUsage, .userPresence], &failure)
        else { throw StepUpError.noPasscode }
        privateAttributes[kSecAttrAccessControl as String] = control
        attributes[kSecAttrTokenID as String] = kSecAttrTokenIDSecureEnclave
        #endif
        attributes[kSecPrivateKeyAttrs as String] = privateAttributes
        var made: Unmanaged<CFError>?
        guard let key = SecKeyCreateRandomKey(attributes as CFDictionary, &made),
            let point = SecKeyCopyPublicKey(key), let data = SecKeyCopyExternalRepresentation(point, nil) as Data?
        else { throw StepUpError.keyUnavailable }
        return data
    }

    public func sign(_ statement: Data, reason: String) async throws -> Data {
        let tag = self.tag
        let signer = self
        return try await Task.detached(priority: .userInitiated) {
            let context = LAContext()
            context.touchIDAuthenticationAllowableReuseDuration = 0
            context.localizedReason = reason
            guard let key = signer.privateKey(context: context, tag: tag) else { throw StepUpError.keyUnavailable }
            var failure: Unmanaged<CFError>?
            guard
                let signature = SecKeyCreateSignature(
                    key, .ecdsaSignatureMessageX962SHA256, statement as CFData, &failure) as Data?
            else {
                let code = (failure?.takeRetainedValue() as Error?).map { ($0 as NSError).code }
                // errSecUserCanceled, errSecAuthFailed, and LAError's cancel codes.
                if code == -128 || code == -2 || code == -4 || code == -9 { throw StepUpError.cancelled }
                throw StepUpError.keyUnavailable
            }
            return signature
        }.value
    }

    private func privateKey(context: LAContext?, tag: Data? = nil) -> SecKey? {
        var query: [String: Any] = [
            kSecClass as String: kSecClassKey,
            kSecAttrApplicationTag as String: tag ?? self.tag,
            kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
            kSecReturnRef as String: true,
        ]
        if let context { query[kSecUseAuthenticationContext as String] = context } else {
            // Reading the public half must not prompt.
            let quiet = LAContext()
            quiet.interactionNotAllowed = true
            query[kSecUseAuthenticationContext as String] = quiet
        }
        var found: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &found) == errSecSuccess else { return nil }
        // swiftlint:disable:next force_cast
        return (found as! SecKey)
    }

    /// Forget the key (a device removed from its runner, or a reset).
    public func deleteKey() {
        var query: [String: Any] = [kSecClass as String: kSecClassKey, kSecAttrApplicationTag as String: tag]
        SecItemDelete(query as CFDictionary)
    }
}
#endif
