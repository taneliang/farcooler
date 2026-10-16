import Foundation
import Security

/// Where the session tokens live.
///
/// The Keychain, and this file exists because they did not always. They were in
/// `UserDefaults`, which on macOS is a plaintext plist under
/// `~/Library/Preferences` readable by anything running as you, and on iOS is a
/// file in the app container that lands in unencrypted local backups. The
/// refresh token is the durable credential — `/v1/auth/refresh` will trade it
/// for a fresh session indefinitely — so reading that plist was account
/// takeover, no exploit required.
///
/// The rest of the account (the user id, the email) stays in `UserDefaults`.
/// Those are labels, not credentials, and they are what the sign-in row shows
/// before anything asks the Keychain for anything.
enum TokenStore {
    /// After first unlock, this device only.
    ///
    /// - `AfterFirstUnlock` rather than `WhenUnlocked`: the daemon's
    ///   notifications arrive while the phone is in a pocket, and the tap that
    ///   opens the app has to be able to refresh a session without the Keychain
    ///   refusing because the screen was locked a moment ago.
    /// - `ThisDeviceOnly` because these tokens are bound to a device
    ///   registration. Syncing them to iCloud would put the credential on
    ///   hardware that never signed in, for no benefit — signing in on a second
    ///   device is a browser tap, and the browser already knows you.
    ///
    /// Computed rather than stored because the `kSecAttr…` constants are
    /// `CFString`, which is not `Sendable`, and a static `let` of one is a
    /// concurrency error under Swift 6 despite the value never changing.
    private static var accessibility: CFString {
        kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    }

    static func read(_ key: String) -> String? {
        var result: CFTypeRef?
        let status = SecItemCopyMatching(
            [
                kSecClass: kSecClassGenericPassword,
                kSecAttrService: service,
                kSecAttrAccount: key,
                kSecReturnData: true,
                kSecMatchLimit: kSecMatchLimitOne,
            ] as CFDictionary, &result)
        guard status == errSecSuccess, let data = result as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Written as an update, falling back to an add when the item is missing.
    ///
    /// It used to be delete-then-add, which made a refused write destroy the
    /// old item as well: a refresh whose new token the Keychain would not take
    /// left no token at all (ov-505). An update that fails leaves what was
    /// there.
    @discardableResult
    static func write(_ key: String, _ value: String) -> Bool {
        writeStatus(key, value) == errSecSuccess
    }

    /// ``write(_:_:)`` with the `OSStatus` kept, for the diagnostic.
    static func writeStatus(_ key: String, _ value: String) -> OSStatus {
        let match =
            [
                kSecClass: kSecClassGenericPassword,
                kSecAttrService: service,
                kSecAttrAccount: key,
            ] as CFDictionary
        let updated = SecItemUpdate(
            match,
            [kSecValueData: Data(value.utf8), kSecAttrAccessible: accessibility] as CFDictionary)
        if updated != errSecItemNotFound { return updated }
        return SecItemAdd(
            [
                kSecClass: kSecClassGenericPassword,
                kSecAttrService: service,
                kSecAttrAccount: key,
                kSecAttrAccessible: accessibility,
                kSecValueData: Data(value.utf8),
            ] as CFDictionary, nil)
    }

    static func delete(_ key: String) {
        SecItemDelete(
            [
                kSecClass: kSecClassGenericPassword,
                kSecAttrService: service,
                kSecAttrAccount: key,
            ] as CFDictionary)
    }

    /// Move anything an older build left in `UserDefaults`, then delete it.
    ///
    /// Not optional and not deferred. A build that only writes to the Keychain
    /// from now on leaves the old plaintext copy sitting there forever, which is
    /// the whole vulnerability preserved for everyone who had already signed in
    /// — the group that matters most.
    static func migrateFromDefaults(_ keys: [String], defaults: UserDefaults) {
        for key in keys {
            guard let value = defaults.string(forKey: key) else { continue }
            // Only forget the plaintext once the Keychain has it. Deleting
            // first would sign out anyone whose Keychain write happened to
            // fail, and the recovery is the same either way.
            if write(key, value) {
                defaults.removeObject(forKey: key)
            }
        }
    }

    /// Distinct from the SSH identity's service so a sign-out cannot reach the
    /// device key, which is a different credential with a different lifetime.
    ///
    /// **One per channel on the Mac** (ov-501). A Mac app with no
    /// keychain-access-groups entitlement writes to the login keychain, where
    /// an item is found by service and account alone, so Far Cooler Canary and
    /// Far Cooler Local shared one `account.refresh`. Each channel is its own
    /// WorkOS environment, so whichever signed in last left the other holding
    /// a session minted for somebody else's client: the relay refused its
    /// access token, refused its refresh token, and Settings said "HTTP 401
    /// after refresh" until the owner signed in again, which only moved the
    /// problem to the other app. The shared item on the owner's Mac was
    /// last written on Sep 4, and every refresh since had failed.
    ///
    /// Stable keeps the name it shipped with, so a release build's session
    /// survives the update. iOS keeps it everywhere: there each app has its own
    /// keychain access group, so the channels never met, and renaming the
    /// service would only sign out every beta phone.
    static var service: String {
        #if os(macOS)
            service(for: AppVersion.channel)
        #else
            sharedService
        #endif
    }

    /// The Mac's name for one channel's session.
    static func service(for channel: String) -> String {
        channel == "stable" ? sharedService : "\(sharedService).\(channel)"
    }

    /// What every channel used before ov-501, and what stable still uses.
    static let sharedService = "com.farcooler.account"
}
