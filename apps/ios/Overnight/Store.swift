import Foundation
import OvernightClient
import Security

/// The device's SSH identity and the hosts it knows.
///
/// The private key lives in the Keychain, not in UserDefaults and not in a
/// file: the Keychain is the only store on iOS that survives a backup restore
/// onto a different device without carrying the secret with it, and the only
/// one an attacker with the app's container cannot simply read.
///
/// `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` is deliberate on both
/// halves. *AfterFirstUnlock* so a background refresh works while the phone is
/// in a pocket; *ThisDeviceOnly* so the key is never in an iCloud backup and a
/// restore onto a new phone produces a device that has to be authorised
/// separately — which is the behaviour you want the day a phone is lost.
enum Identity {
    private static let service = "com.overnight.ssh-key"
    private static let account = "device"

    /// The device's private key, generating one on first use.
    static func privateKey() -> String? {
        if let existing = read() { return existing }
        guard let pair = generate() else { return nil }
        write(pair.privateKey)
        UserDefaults.standard.set(pair.publicKey, forKey: "publicKey")
        return pair.privateKey
    }

    /// The public key to paste into a host's `authorized_keys`.
    static var publicKey: String? {
        _ = privateKey()  // ensure one exists
        return UserDefaults.standard.string(forKey: "publicKey")
    }

    private static func generate() -> (privateKey: String, publicKey: String)? {
        let name = UIDeviceName()
        var buffer = [UInt8](repeating: 0, count: 4096)
        let written = name.withCString { comment in
            buffer.withUnsafeMutableBufferPointer {
                overnight_client_generate_key(comment, $0.baseAddress, $0.count)
            }
        }
        guard written > 0, written <= buffer.count else { return nil }

        guard
            let object = try? JSONSerialization.jsonObject(
                with: Data(buffer[0..<written])) as? [String: Any],
            let priv = object["private_key"] as? String,
            let pub = object["public_key"] as? String
        else { return nil }
        return (priv, pub)
    }

    private static func read() -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
            let data = item as? Data
        else { return nil }
        return String(data: data, encoding: .utf8)
    }

    private static func write(_ key: String) {
        let base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(base as CFDictionary)

        var insert = base
        insert[kSecValueData as String] = Data(key.utf8)
        insert[kSecAttrAccessible as String] =
            kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(insert as CFDictionary, nil)
    }
}

/// A host this device knows how to reach.
struct Host: Codable, Identifiable, Hashable {
    var id: UUID = UUID()
    var label: String
    var address: String
    var port: Int = 22
    var user: String
    /// The host key we have accepted. Nil means we have never connected, and
    /// the first attempt will report the fingerprint rather than trusting it.
    var fingerprint: String?

    /// The JSON the client core expects.
    func config(privateKey: String) -> [String: Any] {
        var config: [String: Any] = [
            "host": address,
            "port": port,
            "user": user,
            "private_key": privateKey,
        ]
        if let fingerprint { config["host_fingerprint"] = fingerprint }
        return config
    }
}

/// Known hosts. Plain UserDefaults: none of this is secret, and the one thing
/// that is lives in the Keychain.
@MainActor
final class HostStore: ObservableObject {
    @Published private(set) var hosts: [Host] = []
    private let key = "hosts"

    init() {
        if let data = UserDefaults.standard.data(forKey: key),
            let decoded = try? JSONDecoder().decode([Host].self, from: data)
        {
            hosts = decoded
        }
    }

    func add(_ host: Host) {
        hosts.append(host)
        save()
    }

    func remove(_ host: Host) {
        hosts.removeAll { $0.id == host.id }
        save()
    }

    /// Record the fingerprint a user has approved.
    func trust(_ host: Host, fingerprint: String) {
        guard let index = hosts.firstIndex(where: { $0.id == host.id }) else { return }
        hosts[index].fingerprint = fingerprint
        save()
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(hosts) else { return }
        UserDefaults.standard.set(data, forKey: key)
    }
}
