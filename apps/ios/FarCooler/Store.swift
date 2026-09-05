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
/// restore onto a new phone produces a device that has to be authorized
/// separately — which is the behavior you want the day a phone is lost.
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
    /// with one key while showing a human a different one to authorize. Every
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
        // it to authorize this device, and it has no way to ask the app.
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
    /// call, authenticated with one, and displayed another for you to authorize.
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
        // Cleared on success, not merely set on failure.
        //
        // It only ever recorded failures, so the value was permanent: nothing
        // removed it once the entitlement that caused it was in place. A
        // simulator that had been broken weeks ago still had -34018 sitting in
        // its preferences, and a night went into re-deriving a diagnosis from
        // it while the keychain underneath was writing and reading perfectly.
        // A field that says "this is broken" and never says "it is not" is not
        // a diagnostic, it is a rumor.
        if status == errSecSuccess {
            UserDefaults.standard.removeObject(forKey: "keychainWriteStatus")
        } else {
            UserDefaults.standard.set(Int(status), forKey: "keychainWriteStatus")
        }
        return status
    }
}

/// This device's tailcat node key pair — the identity a tunneled runner admits.
///
/// A node key is not an SSH key and does not replace one. ``Identity`` above is
/// how a runner decides this device may log in; this is how the TUNNEL decides
/// this device may reach the runner at all. A runner with no address on any
/// network this phone can see is unreachable without one, which is why nothing
/// could ever grant a phone a tunneled runner before this existed.
///
/// **Minted here and never received.** The pair comes out of
/// `farcooler_client_mint_node_key` by value, not out of a file and not off the
/// wire: a private key that exists in two places is not an identity, and the
/// runner only ever learns the public half. The C entry point deliberately
/// takes no path so that this decision cannot be reversed quietly — the private
/// half goes in the Keychain for the reason written at the top of this file.
///
/// **Once per device, not once per runner.** Ten tunneled runners are ten
/// tokens and one node key. `RunnerStore` in `crates/cli/src/runner_pipe.rs`
/// holds it the same way for the desktop, and for the same reason: it is a fact
/// about this device, not about any runner.
///
/// **Both halves are stored, in ONE Keychain item.** ``Identity/publicKey``
/// derives its public half every time and stores nothing, because storing it
/// separately made two facts that diverge — a keychain and a preferences file
/// do not have the same lifetime. There is no entry point that derives a node
/// public key from a node private key, so the pair is kept together, as one
/// value, under one key, with one lifetime. Two facts that cannot outlive each
/// other cannot disagree.
enum NodeIdentity {
    private static let service = "com.farcooler.node-key"
    private static let account = "device"

    /// One mint at a time, for the reason ``Identity/lock`` gives: two callers
    /// finding nothing and both minting would leave the device offering one
    /// public half while holding the other's private one, and a tunnel ignores
    /// a client it does not recognize in silence.
    private static let lock = NSLock()

    /// The pair this device offers, minting one the first time it is needed.
    ///
    /// Nil when this build cannot mint — `{"error":"no_tailcat"}`, which is
    /// every simulator build, because `scripts/build-ios-frameworks.sh` links
    /// the tunnel archive into the device slice only. **That is not a failure
    /// to report.** The answer to it is an offer carrying no node key, which is
    /// the `v=1` shape the core has always accepted, and an enrolment that
    /// produces direct runners exactly as it did before any of this existed.
    static func mintIfNeeded() -> (privateKey: String, publicKey: String)? {
        lock.lock()
        defer { lock.unlock() }
        // Re-read inside the lock: whoever held it may have just minted one.
        if let existing = read() { return existing }
        guard let pair = mint() else { return nil }
        write(pair)
        return pair
    }

    /// The public half to put in an offer, or nil when this device has none.
    static var offeredPublicKey: String? { mintIfNeeded()?.publicKey }

    /// The private half a dial needs — read, never minted.
    ///
    /// Dialing must not mint. A tunneled runner was granted against ONE public
    /// half, which is now a line in that runner's allowlist; minting a second
    /// pair here would produce a key nobody has authorized, and tailcat ignores
    /// an unrecognized client without saying so, so the symptom would be a
    /// connection that hangs and then times out. Nil is the honest answer, and
    /// ``Connection`` turns it into a sentence.
    static var storedPrivateKey: String? {
        lock.lock()
        defer { lock.unlock() }
        return read()?.privateKey
    }

    private static func mint() -> (privateKey: String, publicKey: String)? {
        var buffer = [UInt8](repeating: 0, count: 256)
        let written = buffer.withUnsafeMutableBufferPointer {
            farcooler_client_mint_node_key($0.baseAddress, $0.count)
        }
        guard written > 0, written <= buffer.count else { return nil }
        guard
            let object = try? JSONSerialization.jsonObject(
                with: Data(buffer[0..<written])) as? [String: Any]
        else { return nil }
        // `{"error":"no_tailcat"}` has no `private_key`, so this one guard
        // covers both shapes the entry point can answer with.
        guard
            let priv = object["private_key"] as? String, !priv.isEmpty,
            let pub = object["public_key"] as? String, !pub.isEmpty
        else { return nil }
        return (priv, pub)
    }

    private static func read() -> (privateKey: String, publicKey: String)? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
            let data = item as? Data,
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let priv = object["private_key"] as? String, !priv.isEmpty,
            let pub = object["public_key"] as? String, !pub.isEmpty
        else { return nil }
        return (priv, pub)
    }

    /// Store the pair, and say so if it could not be stored — the same rule
    /// ``Identity/write(_:)`` follows, and for the same reason: a swallowed
    /// keychain status made the app mint a new identity on every call.
    @discardableResult
    private static func write(_ pair: (privateKey: String, publicKey: String)) -> OSStatus {
        guard
            let data = try? JSONSerialization.data(
                withJSONObject: ["private_key": pair.privateKey, "public_key": pair.publicKey])
        else { return errSecParam }

        let base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(base as CFDictionary)

        var insert = base
        insert[kSecValueData as String] = data
        insert[kSecAttrAccessible as String] =
            kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(insert as CFDictionary, nil)
        if status == errSecSuccess {
            UserDefaults.standard.removeObject(forKey: "nodeKeychainWriteStatus")
        } else {
            UserDefaults.standard.set(Int(status), forKey: "nodeKeychainWriteStatus")
        }
        return status
    }
}

/// How a runner is reached: an address, or the tunnel.
///
/// One or the other and never both — two optional fields would admit "both set"
/// and "neither set", and then something here would have to pick a winner. It
/// is the same split `crates/client`'s `parse_destination` makes: a config
/// naming a token and a host would leave two paths to one runner with nothing
/// choosing between them.
///
/// The wire is tagged on `kind` so a third kind is additive, and an
/// unrecognized one throws rather than decoding to a default. This is both what
/// a ceremony reply carries and what ``Runner`` persists — ONE type, because
/// the two are the same fact, and translating between two spellings of it is
/// how a token ends up in a field meant for a hostname.
enum Reach: Codable, Hashable {
    case direct(host: String, port: Int)
    case tailcat(token: String)

    private enum Field: String, CodingKey {
        case kind, host, port, token
    }

    init(from decoder: Decoder) throws {
        let wire = try decoder.container(keyedBy: Field.self)
        switch try wire.decode(String.self, forKey: .kind) {
        case "direct":
            self = .direct(
                host: try wire.decode(String.self, forKey: .host),
                port: try wire.decode(Int.self, forKey: .port))
        case "tailcat":
            self = .tailcat(token: try wire.decode(String.self, forKey: .token))
        case let other:
            throw DecodingError.dataCorruptedError(
                forKey: .kind, in: wire, debugDescription: "unknown reach \(other)")
        }
    }

    func encode(to encoder: Encoder) throws {
        var wire = encoder.container(keyedBy: Field.self)
        switch self {
        case .direct(let host, let port):
            try wire.encode("direct", forKey: .kind)
            try wire.encode(host, forKey: .host)
            try wire.encode(port, forKey: .port)
        case .tailcat(let token):
            try wire.encode("tailcat", forKey: .kind)
            try wire.encode(token, forKey: .token)
        }
    }

    /// The second line under a runner's name.
    ///
    /// A tunnel has no address to show and its token is the one field here
    /// worth stealing — long, meaningless to a person, and a thing to keep off
    /// a screen — so what a person gets is the fact: this one goes through the
    /// tunnel. The user still appears, because which account you log in as is
    /// the other half of what the line is for.
    func detail(user: String) -> String {
        switch self {
        case .direct(let host, _): return "\(user)@\(host)"
        case .tailcat: return "\(user), through the tunnel"
        }
    }

    /// A name for a sentence, when no label was given.
    func name(user: String) -> String {
        switch self {
        case .direct(let host, _): return "\(user)@\(host)"
        case .tailcat: return "a tunneled runner"
        }
    }
}

extension Runner {
    /// A runner supplied at launch, for trying the app against a box you own.
    ///
    /// `-farcoolerDemoHost user@address:port`, which `UserDefaults` exposes for
    /// free: any `-key value` pair on the command line becomes a default in the
    /// argument domain, above everything on disk.
    ///
    /// It exists because the app is useless without a runner and getting one
    /// normally means turning on Remote Login and copying a key between two
    /// screens. This grants nothing — a runner entry is only an address, and the
    /// device still has to be authorized on the far end before it can connect —
    /// and it is not persisted, so removing the argument removes the runner.
    ///
    /// `scripts/demo-host.sh` is what passes it.
    static func fromLaunchArgument() -> Runner? {
        guard let value = UserDefaults.standard.string(forKey: "farcoolerDemoHost"),
            let (user, rest) = split(value, on: "@")
        else { return nil }

        let (address, port) = split(rest, on: ":").map { ($0.0, Int($0.1) ?? 22) } ?? (rest, 22)
        return Runner(
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

/// A runner this device knows how to reach.
///
/// One `farcoolerd`: a Unix user on a host, with its own worktrees. Two entries
/// may name the same box under different users, and they share nothing — which
/// is why this is a runner rather than a machine.
struct Runner: Codable, Identifiable, Hashable {
    var id: UUID = UUID()
    var label: String
    /// An address, or the tunnel. Not an `address` and a `port`, because a
    /// tunneled runner has neither — it has a token — and this app could not
    /// keep one at all until this field existed.
    var reach: Reach
    var user: String
    /// The host key we have accepted — the box's key, not the runner's. Nil
    /// means we have never connected, and
    /// the first attempt will report the fingerprint rather than trusting it.
    var fingerprint: String?

    /// A runner named by address, which is every runner anybody types in.
    init(
        id: UUID = UUID(), label: String, address: String, port: Int = 22, user: String,
        fingerprint: String? = nil
    ) {
        self.init(
            id: id, label: label, reach: .direct(host: address, port: port), user: user,
            fingerprint: fingerprint)
    }

    init(
        id: UUID = UUID(), label: String, reach: Reach, user: String, fingerprint: String? = nil
    ) {
        self.id = id
        self.label = label
        self.reach = reach
        self.user = user
        self.fingerprint = fingerprint
    }

    /// The address, when there is one. Empty for a tunneled runner, which is
    /// why nothing a person reads should be built out of this — see
    /// ``Runner/named``.
    var address: String {
        if case .direct(let host, _) = reach { return host }
        return ""
    }

    var port: Int {
        if case .direct(_, let port) = reach { return port }
        return 22
    }

    /// What to call this runner in a sentence.
    ///
    /// An address for a direct runner, so every sentence this app already
    /// wrote is unchanged — the common path must not move. A tunneled runner
    /// has no address, so it is named by the label somebody ticked on the
    /// device that granted it.
    var named: String {
        switch reach {
        case .direct(let host, _): return host
        case .tailcat: return label.isEmpty ? "this runner" : label
        }
    }

    /// The JSON the client core expects.
    ///
    /// A tunneled runner names its `token` and this DEVICE's node private key
    /// where a direct one names a `host` and a `port`: a token is not an
    /// address, and `parse_destination` refuses to hold both. `nodeKey` is
    /// per device rather than per runner and never travelled in the manifest —
    /// whoever dials supplies the key it already holds.
    func config(privateKey: String, nodeKey: String?) -> [String: Any] {
        var config: [String: Any] = [
            "user": user,
            "private_key": privateKey,
        ]
        switch reach {
        case .direct(let host, let port):
            config["host"] = host
            config["port"] = port
        case .tailcat(let token):
            config["token"] = token
            // Empty rather than absent when this device holds none: absent
            // would leave a config with neither a token's key nor a host, and
            // the core's message would name the wrong missing thing. The
            // caller refuses before it gets here — see ``Connection``.
            config["node_key"] = nodeKey ?? ""
        }
        if let fingerprint { config["host_fingerprint"] = fingerprint }
        return config
    }

    // MARK: Persistence

    private enum Field: String, CodingKey {
        case id, label, reach, user, fingerprint
        // What a runner stored before `reach` existed. Read, never written.
        case address, port
    }

    /// Decoded by hand so that runners saved before `reach` existed still
    /// load.
    ///
    /// `RunnerStore` decodes the whole list in one call, so a single entry the
    /// synthesized decoder threw on would not lose one runner — it would lose
    /// every runner anybody had ever added, silently, on the first launch after
    /// an update. An `address` and a `port` with no `reach` beside them is
    /// exactly what every install on disk today holds, and it means `.direct`.
    init(from decoder: Decoder) throws {
        let stored = try decoder.container(keyedBy: Field.self)
        id = try stored.decodeIfPresent(UUID.self, forKey: .id) ?? UUID()
        label = try stored.decode(String.self, forKey: .label)
        user = try stored.decode(String.self, forKey: .user)
        fingerprint = try stored.decodeIfPresent(String.self, forKey: .fingerprint)
        if let reach = try stored.decodeIfPresent(Reach.self, forKey: .reach) {
            self.reach = reach
        } else {
            reach = .direct(
                host: try stored.decode(String.self, forKey: .address),
                port: try stored.decodeIfPresent(Int.self, forKey: .port) ?? 22)
        }
    }

    /// Written in the new shape only. `address` and `port` are not emitted
    /// beside it: a second copy of where a runner lives is a second fact that
    /// can disagree with the first, which is the mistake
    /// ``Identity/publicKey`` exists to explain.
    func encode(to encoder: Encoder) throws {
        var stored = encoder.container(keyedBy: Field.self)
        try stored.encode(id, forKey: .id)
        try stored.encode(label, forKey: .label)
        try stored.encode(reach, forKey: .reach)
        try stored.encode(user, forKey: .user)
        try stored.encodeIfPresent(fingerprint, forKey: .fingerprint)
    }
}

/// Known runners. Plain UserDefaults: none of this is secret, and the one thing
/// that is lives in the Keychain.
///
/// The defaults keys keep their old spelling on purpose: they name slots on disk
/// that existing installs already wrote, and renaming one would silently forget
/// every runner anybody had added.
@MainActor
final class RunnerStore: ObservableObject {
    @Published private(set) var hosts: [Runner] = []
    private let key = "hosts"
    private let lastKey = "hosts.last"

    /// The runner the app opens onto.
    ///
    /// Persisted because the phone's home screen is now the terminals on a
    /// runner rather than a list of runners — see `RootView`. Landing on
    /// whichever runner happened to be first in the list would mean the app
    /// forgets where you were every time you close it.
    @Published var selected: Runner? {
        didSet { UserDefaults.standard.set(selected?.id.uuidString, forKey: lastKey) }
    }

    init() {
        if let data = UserDefaults.standard.data(forKey: key),
            let decoded = try? JSONDecoder().decode([Runner].self, from: data)
        {
            hosts = decoded
        }
        if let demo = Runner.fromLaunchArgument() {
            // Not saved. It exists for as long as the app was launched with the
            // argument and vanishes without it, so there is nothing to clean up
            // and no way to be left with a runner you did not add.
            hosts.append(demo)
        }

        // Whatever was open last, or the first host. Assigned directly rather
        // than through the property so opening the app does not count as
        // choosing — `didSet` would rewrite the same value back.
        let remembered = UserDefaults.standard.string(forKey: lastKey)
        selected = hosts.first { $0.id.uuidString == remembered } ?? hosts.first
    }

    func add(_ host: Runner) {
        hosts.append(host)
        // Added means wanted: a runner you just typed in is the one you want
        // to be looking at, and the app opens onto whatever is selected.
        selected = host
        save()
    }

    /// Correct a host that was typed in wrong.
    ///
    /// The reason this exists is that a host you cannot connect to is a host you
    /// cannot get past — the app opens onto it — so a mistyped address used to
    /// be permanent, and the app's own screens gave no way to fix or delete it.
    ///
    /// Clears the pinned fingerprint when the host the pin was ABOUT changes.
    /// A fingerprint is a promise about one host at one address; carrying it
    /// across to a corrected address would meet the new host with a changed-key
    /// warning describing a host nobody ever trusted.
    ///
    /// Compared on the whole reach rather than on an address and a port, so a
    /// runner re-pointed from an address to the tunnel — or from one token to
    /// another — drops its pin too. Those are the same event: the box on the
    /// other end is no longer the box the pin was taken from.
    func update(_ host: Runner) {
        guard let index = hosts.firstIndex(where: { $0.id == host.id }) else { return }
        var edited = host
        let previous = hosts[index]
        if edited.reach != previous.reach {
            edited.fingerprint = nil
        }
        hosts[index] = edited
        // Kept in step deliberately: `selected` is a copy, and `RootView` keys
        // the whole screen on it, so this is what makes an edit reconnect
        // instead of leaving the old connection running under new details.
        if selected?.id == edited.id { selected = edited }
        save()
    }

    /// Forget a host key we pinned, so the next connection asks about it again.
    ///
    /// The only honest answer to "this key is not the one recorded". Either the
    /// host was rebuilt, in which case the new key is fine and someone should
    /// look at its fingerprint and say so, or it is an interception, in which
    /// case nothing this app offers should quietly paper over it. Both roads go
    /// through the approval screen, which is where this leads.
    ///
    /// Does NOT touch `selected`: the value there carries no fingerprint of its
    /// own worth preserving, and reassigning it would rebuild the screen out
    /// from under the reconnection this is about to trigger.
    func forgetKey(_ host: Runner) {
        guard let index = hosts.firstIndex(where: { $0.id == host.id }) else { return }
        hosts[index].fingerprint = nil
        save()
    }

    func remove(_ host: Runner) {
        hosts.removeAll { $0.id == host.id }
        if selected?.id == host.id { selected = hosts.first }
        save()
    }

    /// Record the fingerprint a user has approved.
    ///
    /// `selected` is left alone on purpose. `RootView` keys its whole hierarchy
    /// on the selected host, so writing the fingerprint back there would tear
    /// down and rebuild the connection at the exact moment approval succeeded —
    /// the one moment it must not. The caller reconnects with its own approved
    /// copy, and the next launch reads the saved fingerprint back out of `hosts`.
    func trust(_ host: Runner, fingerprint: String) {
        guard let index = hosts.firstIndex(where: { $0.id == host.id }) else { return }
        hosts[index].fingerprint = fingerprint
        save()
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(hosts) else { return }
        UserDefaults.standard.set(data, forKey: key)
    }
}
