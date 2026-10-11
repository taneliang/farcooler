import AgentKit
import Foundation
import OSLog
import Security

/// This Mac's tunnel identity: the node key a tunneled runner admits.
///
/// A node key is not an SSH key. Key A and Key B decide whether sshd lets this
/// Mac log in; this decides whether the TUNNEL lets it reach the runner at all,
/// and a tunnel ignores a client it does not recognize in silence, so a Mac
/// without one cannot be granted a tunneled runner.
///
/// **Two places, one of them the source.** The Keychain holds the pair. A
/// `ProxyCommand` (`farcooler runner pipe`) runs with no app around it, so it
/// reads `runners.json` in the runtime directory instead — a copy, written 0600
/// by `farcooler runner store-node-key` every time this offers the key, so a
/// deleted or stale copy heals the next time the Add This Mac sheet opens. The
/// copy never decides anything: if the two ever differ, the Keychain is right.
///
/// **Minted by the bundled CLI, not by the app.** The app links the client core
/// without the Go archive (the iOS app gets it through an xcframework this one
/// does not use), so `farcooler_client_mint_node_key` here answers `no_tailcat`.
/// The CLI is built with the archive (`apps/macos/build-app.sh`), and
/// `runner mint-node-key` prints the pair once, to us, writing nothing.
///
/// **Not offered yet.** ``offersKey`` is false: this Mac mints and keeps its key,
/// but the offer carries none until phase 3 of ov-481 can use a tunnel. With a
/// key in the offer, a granting Mac swaps a working LAN address for a tunnel
/// token this Mac cannot dial, and every runner it touches starts a tunnel.
/// Phase 3 flips the flag in the commit that adds the grant-time writer.
///
/// **Where the key is not protected like a Keychain item.** The copy in
/// `runners.json` is a 0600 file in the support directory, readable by any
/// process of this user and included in Time Machine, like `~/.ssh/id_*`.
///
/// **No offer without both places.** The iPhone's rule is to offer the key even
/// when its Keychain refused it, because it can say so afterwards. A Mac that
/// did would be granted a tunnel it could not dial and could not explain. So
/// this answers nil — the `v=1` offer, which grants addresses only — unless the
/// pair is in the Keychain AND the copy a dial reads has been written.
actor MacNodeIdentity {
    static let shared = MacNodeIdentity(vault: KeychainNodeKeyVault(), tool: CLINodeKeyTool())

    private let vault: NodeKeyVault
    private let tool: NodeKeyTool
    private var inFlight: Task<String?, Never>?

    init(vault: NodeKeyVault, tool: NodeKeyTool) {
        self.vault = vault
        self.tool = tool
    }

    /// The public half to put in an offer, or nil when this Mac cannot dial.
    ///
    /// Concurrent callers share one attempt. Two that each found the Keychain
    /// empty and each minted would leave the offer showing one public half while
    /// the Keychain held the other's private one.
    func offeredPublicKey() async -> String? {
        if let running = inFlight { return await running.value }
        let attempt = Task { await self.resolve() }
        inFlight = attempt
        let answer = await attempt.value
        inFlight = nil
        return answer
    }

    /// Whether an offer carries the key. See the type's note; false until phase 3.
    static let offersKey = false

    /// Mint and keep the key if this Mac has none, and say nothing about it.
    /// Run when the sheet opens so the key is in place by the time phase 3
    /// offers it.
    func prepare() async { _ = await offeredPublicKey() }

    /// The key an offer carries: nil while ``offersKey`` is false, whatever the
    /// Keychain holds.
    func publicKeyForOffer(offering: Bool = MacNodeIdentity.offersKey) async -> String? {
        guard offering else { return nil }
        return await offeredPublicKey()
    }

    private func resolve() async -> String? {
        let pair: NodeKeyPair
        switch vault.read() {
        case .found(let held):
            pair = held
        case .absent:
            guard let minted = await tool.mint(), vault.write(minted) else { return nil }
            pair = minted
        case .failed:
            // Locked, denied, or an item that will not decode. Minting now
            // would delete the old pair and orphan every runner admitting it.
            return nil
        }
        guard await tool.keep(privateKey: pair.privateKey) else { return nil }
        return pair.publicKey
    }
}

/// Found, absent, or could not be read. Only absent may be minted over.
enum NodeKeyRead: Equatable {
    case found(NodeKeyPair)
    case absent
    case failed
}

struct NodeKeyPair: Equatable {
    var privateKey: String
    var publicKey: String
}

/// Where the pair lives. A protocol so the rule above is tested without
/// touching anybody's Keychain.
protocol NodeKeyVault: Sendable {
    func read() -> NodeKeyRead
    /// True only when the item is stored.
    func write(_ pair: NodeKeyPair) -> Bool
}

/// What the bundled CLI does for us.
protocol NodeKeyTool: Sendable {
    /// A new pair, or nil from a build with no tunnel linked.
    func mint() async -> NodeKeyPair?
    /// Leave the private half where `runner pipe` reads it.
    func keep(privateKey: String) async -> Bool
}

struct KeychainNodeKeyVault: NodeKeyVault {
    /// Channel-scoped, like every other per-install fact: a canary minting over
    /// the release app's key would change who the runners it was granted admit.
    var service = "com.farcooler.node-key"
    var account = AppVersion.channel

    func read() -> NodeKeyRead {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        switch SecItemCopyMatching(query as CFDictionary, &item) {
        case errSecSuccess:
            guard let data = item as? Data, let pair = NodeKeyWire.pair(from: data) else {
                return .failed
            }
            return .found(pair)
        case errSecItemNotFound: return .absent
        default: return .failed
        }
    }

    func write(_ pair: NodeKeyPair) -> Bool {
        let base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(base as CFDictionary)
        var insert = base
        insert[kSecValueData as String] = NodeKeyWire.data(for: pair)
        insert[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        insert[kSecAttrSynchronizable as String] = false
        insert[kSecAttrLabel as String] = "Far Cooler Tunnel Key"
        return SecItemAdd(insert as CFDictionary, nil) == errSecSuccess
    }

    /// Test cleanup only.
    func delete() {
        SecItemDelete(
            [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service,
                kSecAttrAccount as String: account,
            ] as CFDictionary)
    }
}

/// The JSON the CLI prints and the Keychain item holds: the same two keys, so
/// the item is what `runner mint-node-key` said.
enum NodeKeyWire {
    static func pair(from data: Data) -> NodeKeyPair? {
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let priv = object["private_key"] as? String, !priv.isEmpty,
            let pub = object["public_key"] as? String, !pub.isEmpty
        else { return nil }
        return NodeKeyPair(privateKey: priv, publicKey: pub)
    }

    static func data(for pair: NodeKeyPair) -> Data {
        (try? JSONSerialization.data(
            withJSONObject: ["private_key": pair.privateKey, "public_key": pair.publicKey]))
            ?? Data()
    }
}

struct CLINodeKeyTool: NodeKeyTool {
    typealias Run = @Sendable (String, [String], Data?) async -> ProcessRunner.Result

    /// Seconds. The CLI does no network work for either command; this is a
    /// guard against a wedged child, not a budget.
    private static let deadline: TimeInterval = 15
    private static let log = Logger(subsystem: "com.farcooler.FarCooler", category: "node-key")

    var binary: @Sendable () -> String? = { CLI.binary }
    var run: Run = { binary, arguments, stdin in
        await ProcessRunner.run(
            binary, arguments, environment: CLI.environment, stdin: stdin,
            deadline: CLINodeKeyTool.deadline)
    }
    /// Where a failure is said. stderr carries no key (the key travels on
    /// stdin and stdout), so it is safe to keep.
    var report: @Sendable (String) -> Void = { log.error("\($0, privacy: .public)") }

    func mint() async -> NodeKeyPair? {
        guard let ran = await attempt(["runner", "mint-node-key"], stdin: nil, doing: "mint")
        else { return nil }
        guard let pair = NodeKeyWire.pair(from: ran.stdout) else {
            // `{"error":"no_tailcat"}` is the one answer a plain build gives.
            report("mint-node-key gave no key: \(Self.text(ran.stdout.prefix(200)))")
            return nil
        }
        return pair
    }

    func keep(privateKey: String) async -> Bool {
        await attempt(
            ["runner", "store-node-key"], stdin: Data(privateKey.utf8), doing: "store") != nil
    }

    private func attempt(_ arguments: [String], stdin: Data?, doing: String)
        async -> ProcessRunner.Result?
    {
        guard let binary = binary() else {
            report("\(doing): no farcooler CLI found")
            return nil
        }
        let ran = await run(binary, arguments, stdin)
        guard ran.succeeded else {
            let why = ran.launchFailure ?? (ran.timedOut ? "timed out" : "exit \(ran.status)")
            report("\(doing) failed: \(why): \(Self.text(ran.stderr.prefix(500)))")
            return nil
        }
        return ran
    }

    private static func text(_ data: some DataProtocol) -> String {
        String(decoding: Data(data), as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
