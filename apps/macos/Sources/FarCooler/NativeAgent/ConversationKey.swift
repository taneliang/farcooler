import AgentKit
import CFarCoolerClient
import Foundation
import Security

/// Where this Mac's conversation key is kept: the Keychain in the app, memory
/// in a test, so no test writes the owner's login keychain.
protocol ConversationKeyVault: Sendable {
    func read() -> ConversationKeyRead
    @discardableResult func write(_ privateKey: String) -> Bool
    func delete()
}

/// What a vault read found. `failed` is not `absent`: a Keychain that's
/// locked or refuses is no reason to make a new key, which would be a new
/// device to every runner and would re-pair runners that revoked the old one.
enum ConversationKeyRead: Equatable, Sendable {
    case found(String)
    case absent
    case failed
}

/// The key this Mac's client core signs in to remote runners with, for the
/// conversation view (ov-408).
///
/// **Not Key A** (`DeviceKey`). Key A is a file, per relay account, because
/// the CLI hands it to `ssh -i`. This one is used only inside the app, by the
/// client core, so it needs no file and no account: it lives in the Keychain,
/// as the phone's `Identity` does. It's never in `~/.ssh`, and `ssh` never
/// offers it for anything.
///
/// Paired with a runner the way a phone is: a restricted, `control`-scoped
/// line written by the runner's daemon through `client.enroll`
/// (`RemotePairing`). Revoked the way a phone is: `client revoke` with its
/// client id, which is derived from the key, never stored.
final class ConversationKey: @unchecked Sendable {
    static let shared = ConversationKey(vault: KeychainConversationKeyVault())

    private let vault: any ConversationKeyVault
    /// One generation at a time, re-reading inside: two first callers finding
    /// nothing and both generating would leave the Mac pairing one key and
    /// signing in with another (`Identity.lock` on the phone).
    private let lock = NSLock()

    init(vault: any ConversationKeyVault) {
        self.vault = vault
    }

    /// The private key, generated and stored the first time it's needed. Nil
    /// when the Keychain won't keep it: a key that can't be kept would be a
    /// new identity on every call.
    func privateKey() -> String? {
        lock.lock()
        defer { lock.unlock() }
        switch vault.read() {
        case .found(let existing): return existing
        case .failed: return nil
        case .absent: break
        }
        guard let generated = Self.generate(), vault.write(generated), vault.read() == .found(generated) else { return nil }
        return generated
    }

    /// The public half, derived from the private key every time.
    var publicKey: String? {
        privateKey().flatMap(DeviceKey.publicKey(of:))
    }

    /// The client id the runner's line carries, derived from the public key by
    /// the core's one rule (`DeviceKey.clientID`).
    var clientID: String? {
        publicKey.flatMap(DeviceKey.clientID(of:))
    }

    private static func generate() -> String? {
        var buffer = [UInt8](repeating: 0, count: 4096)
        let comment = "farcooler-conversation-\(thisMacName)"
        let written = comment.withCString { comment in
            buffer.withUnsafeMutableBufferPointer { farcooler_client_generate_key(comment, $0.baseAddress, $0.count) }
        }
        guard written > 0, written <= buffer.count,
            let object = try? JSONSerialization.jsonObject(with: Data(buffer[0..<written])) as? [String: Any]
        else { return nil }
        return object["private_key"] as? String
    }
}

/// The Keychain: a generic password, never synced, readable without asking
/// only by this app's signature. It's the login keychain, so the
/// after-first-unlock, this-device-only attribute is advisory: the item moves
/// with `login.keychain-db` (Migration Assistant, a restored backup), as the
/// account tokens (`TokenStore`) do. Channel-scoped like everything a channel
/// owns, so a canary and a release build never pair one key.
struct KeychainConversationKeyVault: ConversationKeyVault {
    var service = "com.farcooler.conversation-key.\(AppVersion.channel)"
    private static let account = "device"

    private var query: [CFString: Any] {
        [kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: Self.account]
    }

    func read() -> ConversationKeyRead {
        var ask = query
        ask[kSecReturnData] = true
        ask[kSecMatchLimit] = kSecMatchLimitOne
        var result: CFTypeRef?
        switch SecItemCopyMatching(ask as CFDictionary, &result) {
        case errSecSuccess:
            guard let data = result as? Data, let text = String(data: data, encoding: .utf8), !text.isEmpty else { return .failed }
            return .found(text)
        case errSecItemNotFound: return .absent
        default: return .failed
        }
    }

    @discardableResult
    func write(_ privateKey: String) -> Bool {
        delete()
        var add = query
        add[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        add[kSecAttrSynchronizable] = false
        add[kSecAttrLabel] = "Far Cooler Conversation Key"
        add[kSecValueData] = Data(privateKey.utf8)
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess
    }

    func delete() {
        SecItemDelete(query as CFDictionary)
    }
}

/// Memory, for tests.
final class MemoryConversationKeyVault: ConversationKeyVault, @unchecked Sendable {
    private let lock = NSLock()
    private var stored: String?
    /// How many writes reached the vault: a second means a second key.
    private(set) var writes = 0

    init(_ stored: String? = nil) { self.stored = stored }

    /// Set to make reads fail, as a locked or refusing Keychain does.
    var failing = false

    func read() -> ConversationKeyRead {
        lock.lock()
        defer { lock.unlock() }
        if failing { return .failed }
        return stored.map(ConversationKeyRead.found) ?? .absent
    }

    func write(_ privateKey: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        stored = privateKey
        writes += 1
        return true
    }

    func delete() {
        lock.lock()
        defer { lock.unlock() }
        stored = nil
    }
}
