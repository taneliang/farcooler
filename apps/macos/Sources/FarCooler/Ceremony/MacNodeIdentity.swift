import AgentKit
import Foundation
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
/// deleted or stale copy heals the next time the Add Device sheet opens. The
/// copy never decides anything: if the two ever differ, the Keychain is right.
///
/// **Minted by the bundled CLI, not by the app.** The app links the client core
/// without the Go archive (the iOS app gets it through an xcframework this one
/// does not use), so `farcooler_client_mint_node_key` here answers `no_tailcat`.
/// The CLI is built with the archive (`apps/macos/build-app.sh`), and
/// `runner mint-node-key` prints the pair once, to us, writing nothing.
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

    private func resolve() async -> String? {
        let pair: NodeKeyPair
        if let held = vault.read() {
            pair = held
        } else {
            guard let minted = await tool.mint(), vault.write(minted) else { return nil }
            pair = minted
        }
        guard await tool.keep(privateKey: pair.privateKey) else { return nil }
        return pair.publicKey
    }
}

struct NodeKeyPair: Equatable {
    var privateKey: String
    var publicKey: String
}

/// Where the pair lives. A protocol so the rule above is tested without
/// touching anybody's Keychain.
protocol NodeKeyVault: Sendable {
    func read() -> NodeKeyPair?
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

    func read() -> NodeKeyPair? {
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
        return NodeKeyWire.pair(from: data)
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
    /// Seconds. The CLI does no network work for either command; this is a
    /// guard against a wedged child, not a budget.
    private static let deadline: TimeInterval = 15

    func mint() async -> NodeKeyPair? {
        guard let binary = CLI.binary else { return nil }
        let ran = await ProcessRunner.run(
            binary, ["runner", "mint-node-key"], environment: CLI.environment,
            deadline: Self.deadline, discardStderr: true)
        guard ran.succeeded else { return nil }
        return NodeKeyWire.pair(from: ran.stdout)
    }

    func keep(privateKey: String) async -> Bool {
        guard let binary = CLI.binary else { return false }
        let ran = await ProcessRunner.run(
            binary, ["runner", "store-node-key"], environment: CLI.environment,
            stdin: Data(privateKey.utf8), deadline: Self.deadline, discardStderr: true)
        return ran.succeeded
    }
}
