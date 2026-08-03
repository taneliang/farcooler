import Foundation
import FarCoolerClient
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
    private static let service = "com.farcooler.ssh-key"
    private static let account = "device"

    /// One generation at a time.
    ///
    /// `privateKey()` reads, and generates only if it found nothing — which is
    /// safe exactly once. At launch two callers ask at the same moment: the root
    /// view, to show the key you paste into a host, and a connection, to
    /// authenticate with it. Both could find nothing, both generate, and the
    /// second write replaces the first — so the device authenticated with one key
    /// while displaying another, and every connection was refused with a
    /// correct-looking key on screen.
    ///
    /// A lock rather than an actor: this is called from synchronous SwiftUI and
    /// from the connection path, and making it async would put an `await` in
    /// front of every use of the device's own identity.
    private static let lock = NSLock()

    /// The device's private key, generating one on first use.
    static func privateKey() -> String? {
        lock.lock()
        defer { lock.unlock() }
        // Re-read inside the lock: whoever held it may have just created one.
        if let existing = read() { return existing }
        guard let pair = generate() else { return nil }
        write(pair.privateKey)
        return pair.privateKey
    }

    /// The public key to paste into a host's `authorized_keys`.
    ///
    /// Derived from the private key every time, never stored. It used to be
    /// cached in `UserDefaults` at the moment of generation, which made two
    /// sources for one fact — and they diverge, because the keychain and the
    /// preferences file do not have the same lifetime. A reinstall keeps the
    /// keychain and takes the preferences, so the app went on authenticating
    /// with one key while showing a human a different one to authorise. Every
    /// connection was then refused with a correct-looking key on screen.
    static var publicKey: String? {
        guard let key = privateKey() else { return nil }
        var buffer = [UInt8](repeating: 0, count: 2048)
        let written = key.withCString { text in
            buffer.withUnsafeMutableBufferPointer {
                farcooler_client_public_key(text, $0.baseAddress, $0.count)
            }
        }
        guard written > 0, written <= buffer.count else { return nil }
        let derived = String(decoding: buffer[0..<written], as: UTF8.self)

        // Also written where tooling can read it — `scripts/demo-host.sh` needs
        // it to authorise this device, and it has no way to ask the app.
        //
        // A projection of the private key, refreshed from it on every read, not
        // a second place the answer lives. That distinction is the whole fix:
        // the old code wrote this once at generation and never again, so a
        // keychain that outlived its preferences left a stale key on display
        // while a different one was being offered.
        UserDefaults.standard.set(derived, forKey: "publicKey")
        return derived
    }

    private static func generate() -> (privateKey: String, publicKey: String)? {
        let name = UIDeviceName()
        var buffer = [UInt8](repeating: 0, count: 4096)
        let written = name.withCString { comment in
            buffer.withUnsafeMutableBufferPointer {
                farcooler_client_generate_key(comment, $0.baseAddress, $0.count)
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

    /// Store the key, and say so if it could not be stored.
    ///
    /// The status used to be discarded. A keychain that refuses the write is not
    /// a rare edge: an app built without the entitlement that grants access gets
    /// `errSecMissingEntitlement` every time. And because the failure was
    /// swallowed, the next call found nothing, generated another key, failed to
    /// store that one too, and so on — so the device had a NEW identity on every
    /// call, authenticated with one, and displayed another for you to authorise.
    /// It looked exactly like a host rejecting a correct key.
    @discardableResult
    private static func write(_ key: String) -> OSStatus {
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
        let status = SecItemAdd(insert as CFDictionary, nil)
        if status != errSecSuccess {
            UserDefaults.standard.set(Int(status), forKey: "keychainWriteStatus")
        }
        return status
    }
}

extension Host {
    /// A host supplied at launch, for trying the app against a machine you own.
    ///
    /// `-farcoolerDemoHost user@address:port`, which `UserDefaults` exposes for
    /// free: any `-key value` pair on the command line becomes a default in the
    /// argument domain, above everything on disk.
    ///
    /// It exists because the app is useless without a host and getting one
    /// normally means turning on Remote Login and copying a key between two
    /// screens. This grants nothing — a host entry is only an address, and the
    /// device still has to be authorised on the far end before it can connect —
    /// and it is not persisted, so removing the argument removes the host.
    ///
    /// `scripts/demo-host.sh` is what passes it.
    static func fromLaunchArgument() -> Host? {
        guard let value = UserDefaults.standard.string(forKey: "farcoolerDemoHost"),
            let (user, rest) = split(value, on: "@")
        else { return nil }

        let (address, port) = split(rest, on: ":").map { ($0.0, Int($0.1) ?? 22) } ?? (rest, 22)
        return Host(
            label: "Demo host",
            address: address,
            port: port,
            user: user,
            // The script generates the host key it points at, so there is no
            // human to show a fingerprint to. A real host still gets the
            // approval screen.
            fingerprint: "accept-any")
    }

    private static func split(_ text: String, on separator: Character) -> (String, String)? {
        guard let index = text.lastIndex(of: separator) else { return nil }
        return (String(text[text.startIndex..<index]), String(text[text.index(after: index)...]))
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
    private let lastKey = "hosts.last"

    /// The host the app opens onto.
    ///
    /// Persisted because the phone's home screen is now the terminals on a
    /// machine rather than a list of machines — see `RootView`. Landing on
    /// whichever host happened to be first in the list would mean the app
    /// forgets where you were every time you close it.
    @Published var selected: Host? {
        didSet { UserDefaults.standard.set(selected?.id.uuidString, forKey: lastKey) }
    }

    init() {
        if let data = UserDefaults.standard.data(forKey: key),
            let decoded = try? JSONDecoder().decode([Host].self, from: data)
        {
            hosts = decoded
        }
        if let demo = Host.fromLaunchArgument() {
            // Not saved. It exists for as long as the app was launched with the
            // argument and vanishes without it, so there is nothing to clean up
            // and no way to be left with a host you did not add.
            hosts.append(demo)
        }

        // Whatever was open last, or the first host. Assigned directly rather
        // than through the property so opening the app does not count as
        // choosing — `didSet` would rewrite the same value back.
        let remembered = UserDefaults.standard.string(forKey: lastKey)
        selected = hosts.first { $0.id.uuidString == remembered } ?? hosts.first
    }

    func add(_ host: Host) {
        hosts.append(host)
        // Added means wanted: a host you just typed in is the one you want to
        // be looking at, and the app opens onto whatever is selected.
        selected = host
        save()
    }

    func remove(_ host: Host) {
        hosts.removeAll { $0.id == host.id }
        if selected?.id == host.id { selected = hosts.first }
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
