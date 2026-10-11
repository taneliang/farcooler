import Foundation
import Testing

@testable import Far_Cooler

/// Whether this Mac offers a tunnel key, and the two places it keeps one.
///
/// The rule under test: an offer carries the key only when the pair is in the
/// Keychain AND the copy `runner pipe` reads has been written. Anything less is
/// a Mac granted a tunnel it cannot dial and cannot explain, which is worse than
/// being granted an address. None of these touch the real Keychain (one test uses
/// an item named for itself and removes it) or the real runtime directory.
struct MacNodeIdentityTests {
    static let pair = NodeKeyPair(privateKey: "private-half", publicKey: "public-half")

    /// Counts what happened to it, safely from any task.
    final class Vault: NodeKeyVault, @unchecked Sendable {
        private let lock = NSLock()
        private var held: NodeKeyPair?
        private(set) var writes = 0
        let accepts: Bool
        let unreadable: Bool
        init(held: NodeKeyPair? = nil, accepts: Bool = true, unreadable: Bool = false) {
            self.held = held
            self.accepts = accepts
            self.unreadable = unreadable
        }
        func read() -> NodeKeyRead {
            lock.withLock {
                if unreadable { return .failed }
                return held.map(NodeKeyRead.found) ?? .absent
            }
        }
        func write(_ pair: NodeKeyPair) -> Bool {
            lock.withLock {
                writes += 1
                guard accepts else { return false }
                held = pair
                return true
            }
        }
    }

    final class Tool: NodeKeyTool, @unchecked Sendable {
        private let lock = NSLock()
        private(set) var mints = 0
        private(set) var kept: [String] = []
        let minting: NodeKeyPair?
        let keeps: Bool
        init(minting: NodeKeyPair? = MacNodeIdentityTests.pair, keeps: Bool = true) {
            self.minting = minting
            self.keeps = keeps
        }
        func mint() async -> NodeKeyPair? {
            // A real mint takes a process; yield so a second caller can arrive.
            try? await Task.sleep(nanoseconds: 20_000_000)
            return lock.withLock {
                mints += 1
                return minting
            }
        }
        func keep(privateKey: String) async -> Bool {
            lock.withLock {
                kept.append(privateKey)
                return keeps
            }
        }
    }

    @Test func theFirstOfferMintsKeepsInBothPlacesAndOffersThePublicHalf() async {
        let vault = Vault()
        let tool = Tool()
        let offered = await MacNodeIdentity(vault: vault, tool: tool).offeredPublicKey()

        #expect(offered == "public-half")
        #expect(vault.read() == .found(Self.pair), "the Keychain does not hold the pair it offered")
        #expect(tool.kept == ["private-half"], "the copy a dial reads was not written")
    }

    /// The same key every time: a second offer minting afresh would leave every
    /// runner already granted the first one admitting a key nobody holds.
    @Test func aLaterOfferReusesTheKeyAndRewritesTheCopy() async {
        let vault = Vault(held: Self.pair)
        let tool = Tool(minting: NodeKeyPair(privateKey: "other", publicKey: "other-pub"))
        let offered = await MacNodeIdentity(vault: vault, tool: tool).offeredPublicKey()

        #expect(offered == "public-half")
        #expect(tool.mints == 0, "a key was minted over the one in the Keychain")
        #expect(tool.kept == ["private-half"], "the copy was not refreshed from the Keychain")
    }

    /// A build with no tunnel linked: the v=1 offer, and nothing half-written.
    @Test func aBuildWithNoTunnelOffersNoKeyAndStoresNothing() async {
        let vault = Vault()
        let tool = Tool(minting: nil)
        let offered = await MacNodeIdentity(vault: vault, tool: tool).offeredPublicKey()

        #expect(offered == nil)
        #expect(vault.writes == 0)
        #expect(tool.kept.isEmpty)
    }

    /// The iPhone offers anyway here. A Mac must not: the runner would grant a
    /// tunnel whose key this Mac then could not produce.
    @Test func aKeychainThatRefusesTheKeyMeansNoOffer() async {
        let vault = Vault(accepts: false)
        let tool = Tool()
        let offered = await MacNodeIdentity(vault: vault, tool: tool).offeredPublicKey()

        #expect(offered == nil, "offered a key the Keychain would not keep")
        #expect(tool.kept.isEmpty, "mirrored a key the Keychain refused")
    }

    @Test func aCopyThatCouldNotBeWrittenMeansNoOffer() async {
        let vault = Vault(held: Self.pair)
        let tool = Tool(keeps: false)
        let offered = await MacNodeIdentity(vault: vault, tool: tool).offeredPublicKey()

        #expect(offered == nil, "offered a key `runner pipe` could not read back")
    }

    /// A locked or denied Keychain is not an empty one. Minting here would
    /// delete the old pair and orphan every runner that admits it.
    @Test func aKeychainThatCannotBeReadIsNeverMintedOver() async {
        let vault = Vault(held: Self.pair, unreadable: true)
        let tool = Tool()
        let offered = await MacNodeIdentity(vault: vault, tool: tool).offeredPublicKey()

        #expect(offered == nil)
        #expect(tool.mints == 0, "minted over a key that could not be read")
        #expect(vault.writes == 0, "overwrote a key that could not be read")
        #expect(tool.kept.isEmpty)
    }

    /// Until phase 3, the offer carries no key however many this Mac holds; the
    /// flag is a deliberate one-line flip.
    @Test func theOfferCarriesNoKeyWhileTheFlagIsOff() async {
        #expect(MacNodeIdentity.offersKey == false, "the flag changed: phase 3 must own this")
        let vault = Vault(held: Self.pair)
        let identity = MacNodeIdentity(vault: vault, tool: Tool())
        #expect(await identity.publicKeyForOffer() == nil)
        #expect(await identity.publicKeyForOffer(offering: true) == "public-half")
    }

    /// Minting and storing still happen with the flag off, so phase 3 starts
    /// with the identity in place.
    @Test func preparingStillMintsAndKeepsTheKey() async {
        let vault = Vault()
        let tool = Tool()
        await MacNodeIdentity(vault: vault, tool: tool).prepare()
        #expect(vault.read() == .found(Self.pair))
        #expect(tool.kept == ["private-half"])
    }

    /// Failures are said, with the CLI's own words, and carry no key.
    @Test func aFailedMintOrStoreIsReportedWithItsStderr() async {
        final class Said: @unchecked Sendable {
            let lock = NSLock()
            var lines: [String] = []
        }
        let said = Said()
        var failing = ProcessRunner.Result()
        failing.status = 1
        failing.stderr = Data("runners.json is mode 644".utf8)
        let result = failing
        var tool = CLINodeKeyTool()
        tool.binary = { "/bin/farcooler" }
        tool.run = { _, _, _ in result }
        tool.report = { line in said.lock.withLock { said.lines.append(line) } }

        #expect(await tool.keep(privateKey: "secret-private-half") == false)
        #expect(await tool.mint() == nil)
        let lines = said.lock.withLock { said.lines }
        #expect(lines.count == 2)
        #expect(lines.allSatisfy { $0.contains("runners.json is mode 644") }, "\(lines)")
        #expect(!lines.joined().contains("secret-private-half"))
    }

    @Test func twoOffersAtOnceMintOnce() async {
        let vault = Vault()
        let tool = Tool()
        let identity = MacNodeIdentity(vault: vault, tool: tool)
        async let first = identity.offeredPublicKey()
        async let second = identity.offeredPublicKey()
        let offered = await [first, second]

        #expect(offered == ["public-half", "public-half"])
        #expect(tool.mints == 1, "two callers each minted a key")
    }

    /// The offer the other device scans actually carries the key: the argument
    /// reaches the core and comes out as `node_key`, and nil still means none.
    @Test func theOfferCarriesTheKeyItWasGiven() throws {
        func offer(_ nodeKey: String?) throws -> [String: Any] {
            let text = try #require(
                CeremonyFFI.offer(
                    name: "Test Mac", account: "acct_1", keyA: "ssh-ed25519 AAAA test", keyB: nil,
                    nodeKey: nodeKey))
            return try #require(
                JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any])
        }
        let key = String(repeating: "A", count: 43)
        #expect(try offer(key)["node_key"] as? String == key)
        #expect(try offer(nil)["node_key"] as? String == "")
    }

    /// The Keychain item round-trips the pair. A uniquely named item, removed
    /// afterwards: this is the one test that touches a real Keychain.
    @Test func theKeychainKeepsWhatItWasGiven() {
        let vault = KeychainNodeKeyVault(
            service: "com.farcooler.node-key.test-\(UUID().uuidString)", account: "test")
        defer { vault.delete() }

        #expect(vault.read() == .absent)
        #expect(vault.write(Self.pair))
        #expect(vault.read() == .found(Self.pair))
        let replacement = NodeKeyPair(privateKey: "p2", publicKey: "q2")
        #expect(vault.write(replacement))
        #expect(vault.read() == .found(replacement), "a second write did not replace the first")
    }

    @Test func theWireIsTheCLIsJSON() {
        let printed = #"{"private_key":"a","public_key":"b"}"#
        #expect(NodeKeyWire.pair(from: Data(printed.utf8)) == NodeKeyPair(privateKey: "a", publicKey: "b"))
        #expect(NodeKeyWire.pair(from: Data(#"{"error":"no_tailcat"}"#.utf8)) == nil)
        #expect(NodeKeyWire.pair(from: Data(#"{"private_key":"","public_key":"b"}"#.utf8)) == nil)
    }
}
