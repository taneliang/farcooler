import Foundation
import Testing

@testable import Far_Cooler

/// The address in a granted runner is written into the other device once and
/// never resolved again, so the classifier below is the only thing standing
/// between a phone and a runner that dies at the end of the driveway. It is
/// pure, which means it can be wrong in exactly one place and be wrong
/// everywhere — hence the size of this table.
struct RunnerReachTests {
    // MARK: - Names

    @Test func mdnsNamesGoNoFurtherThanTheLink() {
        // `ProcessInfo.hostName` hands one of these back on almost every Mac,
        // which is how the dead-runner bug was shipped in the first place.
        #expect(RunnerFacts.reach(of: "cosmo.local") == .thisNetwork)
        #expect(RunnerFacts.reach(of: "COSMO.LOCAL") == .thisNetwork)
        // The fully qualified form, root dot and all — `tailscale status`
        // prints names this way and people paste them.
        #expect(RunnerFacts.reach(of: "cosmo.local.") == .thisNetwork)
        #expect(RunnerFacts.reach(of: "  cosmo.local  ") == .thisNetwork)
        #expect(RunnerFacts.reach(of: "local") == .thisNetwork)
    }

    @Test func aNameWithNoDotIsWhateverTheResolverSaysToday() {
        #expect(RunnerFacts.reach(of: "cosmo") == .thisNetwork)
        #expect(RunnerFacts.reach(of: "box") == .thisNetwork)
        // An `ssh_config` alias of `1` is a single-label name, not 0.0.0.1 —
        // `IPv4Address` takes the short `inet_aton` forms and would have said
        // otherwise.
        #expect(RunnerFacts.reach(of: "1") == .thisNetwork)
    }

    /// RFC 8375 reserved this so that it would never resolve outside a home
    /// network, and routers hand it out.
    @Test func homeArpaIsTheOtherNameThatStopsAtTheDoor() {
        #expect(RunnerFacts.reach(of: "nas.home.arpa") == .thisNetwork)
        #expect(RunnerFacts.reach(of: "home.arpa") == .thisNetwork)
    }

    @Test func aMagicDNSNameTravels() {
        #expect(RunnerFacts.reach(of: "cosmo.tail23af.ts.net") == .anywhere)
        #expect(RunnerFacts.reach(of: "cosmo.tail23af.ts.net.") == .anywhere)
        // A two-label tailnet name, which is a real one: `hello.ts.net` is a
        // node in this Mac's own peer list.
        #expect(RunnerFacts.reach(of: "hello.ts.net") == .anywhere)
    }

    @Test func anOrdinaryPublicNameTravels() {
        #expect(RunnerFacts.reach(of: "box.example.com") == .anywhere)
        #expect(RunnerFacts.reach(of: "runner.internal.example.org") == .anywhere)
    }

    // MARK: - IPv4

    @Test func everyPrivateIPv4RangeStaysHome() {
        for address in [
            "10.0.0.1", "10.255.255.255",
            "172.16.0.1", "172.31.255.254",
            "192.168.1.180",
            // Self-assigned: DHCP did not answer, so this is not stable even
            // for the machine that has it.
            "169.254.10.1",
            "127.0.0.1", "127.94.0.2",
        ] {
            #expect(RunnerFacts.reach(of: address) == .thisNetwork, "\(address)")
        }
    }

    /// The two off-by-one neighbours of 172.16.0.0/12, which is the range
    /// people get wrong.
    @Test func theRangesAroundTheTwentyBitBlockAreOrdinaryPublicAddresses() {
        #expect(RunnerFacts.reach(of: "172.15.255.255") == .anywhere)
        #expect(RunnerFacts.reach(of: "172.32.0.1") == .anywhere)
    }

    /// 100.64.0.0/10, RFC 6598 — where Tailscale puts its nodes. This Mac's
    /// own tailnet address is 100.66.207.119.
    @Test func aTailnetIPv4Travels() {
        #expect(RunnerFacts.reach(of: "100.66.207.119") == .anywhere)
        #expect(RunnerFacts.reach(of: "100.64.0.1") == .anywhere)
        #expect(RunnerFacts.reach(of: "100.127.255.254") == .anywhere)
        // Both sides of the block, which are ordinary public space.
        #expect(RunnerFacts.reach(of: "100.63.255.255") == .anywhere)
        #expect(RunnerFacts.reach(of: "100.128.0.1") == .anywhere)
    }

    @Test func aPublicIPv4Travels() {
        #expect(RunnerFacts.reach(of: "104.176.7.64") == .anywhere)
        #expect(RunnerFacts.reach(of: "8.8.8.8") == .anywhere)
    }

    // MARK: - IPv6

    @Test func everyPrivateIPv6RangeStaysHome() {
        for address in [
            "::1",
            "fe80::1", "fe80::1cbd:2c4a:9f0f:8a1b",
            // The zone names an interface on THIS Mac, so it could not travel
            // even if the address did.
            "fe80::1%en0",
            "fc00::1", "fd00::1", "fdff:ffff::1",
            // The bracket form, which is how a literal is written where a bare
            // colon would read as a port.
            "[fe80::1]", "[::1]",
            // IPv4 inside a v6 coat: what decides it is the v4.
            "::ffff:192.168.1.1",
        ] {
            #expect(RunnerFacts.reach(of: address) == .thisNetwork, "\(address)")
        }
    }

    /// fd7a:115c:a1e0::/48 is inside fc00::/7 and is Tailscale's. Every node
    /// carries one beside its 100.x address — `fd7a:115c:a1e0::f3b:cf77` is
    /// this Mac's — and calling it "your Wi-Fi only" would offer to improve on
    /// an address that is already the improvement.
    @Test func aTailnetIPv6Travels() {
        #expect(RunnerFacts.reach(of: "fd7a:115c:a1e0::f3b:cf77") == .anywhere)
        #expect(RunnerFacts.reach(of: "[fd7a:115c:a1e0::f3b:cf77]") == .anywhere)
        #expect(RunnerFacts.reach(of: "fd7a:115c:a1e0:ab12:4843:cd96:6248:347f") == .anywhere)
        // One nibble off the tailnet prefix, and it is unique-local again.
        #expect(RunnerFacts.reach(of: "fd7a:115c:a1e1::1") == .thisNetwork)
    }

    @Test func aPublicIPv6Travels() {
        #expect(RunnerFacts.reach(of: "2600:1700:2800:9210:a4de:8de0:eb8c:f5e") == .anywhere)
        #expect(RunnerFacts.reach(of: "[2600:1700::1]") == .anywhere)
        #expect(RunnerFacts.reach(of: "::ffff:8.8.8.8") == .anywhere)
    }

    // MARK: - Nothing to say

    /// Unknown is a refusal to guess, not a claim that the address fails.
    @Test func anAddressThatIsNotOneReadsAsUnknown() {
        #expect(RunnerFacts.reach(of: "") == .unknown)
        #expect(RunnerFacts.reach(of: "   ") == .unknown)
        #expect(RunnerFacts.reach(of: ".") == .unknown)
        // 0.0.0.0/8 and `::` are not places anything is reached.
        #expect(RunnerFacts.reach(of: "0.0.0.0") == .unknown)
        #expect(RunnerFacts.reach(of: "0.1.2.3") == .unknown)
        #expect(RunnerFacts.reach(of: "::") == .unknown)
        #expect(RunnerFacts.reach(of: "::ffff:0.0.0.0") == .unknown)
    }
}

/// The other half: reading what `tailscale status --json` says, against a
/// captured document rather than against whatever this Mac's tailnet contains
/// today. ``TailscaleFixture/tailnet`` is trimmed out of a real one — client
/// 1.102.1 — with the node keys shortened and everything this file does not
/// read dropped.
struct TailnetTests {
    // MARK: - Reading the document

    @Test func aRunningDaemonYieldsSelfAndItsPeers() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))

        #expect(tailnet.selfNode?.hostName == "Cosmo")
        // The root dot is off: everything downstream compares this against
        // addresses that never have one.
        #expect(tailnet.selfNode?.dnsName == "cosmo.tail23af.ts.net")
        #expect(tailnet.selfNode?.addresses.first == "100.66.207.119")
        #expect(tailnet.peers.count == 7)
        // This Mac is reported separately and is never among its own peers.
        #expect(!tailnet.peers.contains { $0.dnsLabel == "cosmo" })
    }

    /// Signed out, the daemon still hands over a `Self` full of names from the
    /// last tailnet it was on. Offering one of those is worse than offering
    /// nothing.
    @Test func aDaemonThatIsNotLoggedInSaysNothing() {
        let loggedOut = TailscaleFixture.tailnet.replacingOccurrences(
            of: "\"BackendState\": \"Running\"", with: "\"BackendState\": \"NeedsLogin\"")

        #expect(Tailnet(json: TailscaleFixture.data(loggedOut)) == nil)
    }

    /// Every failure is nothing rather than a throw: the Add Device sheet has
    /// to work on a Mac where none of this exists.
    @Test func nothingAboutABadDocumentThrows() {
        #expect(Tailnet(json: Data()) == nil)
        #expect(Tailnet(json: TailscaleFixture.data("not json at all")) == nil)
        #expect(Tailnet(json: TailscaleFixture.data("{}")) == nil)
        #expect(Tailnet(json: TailscaleFixture.data("[]")) == nil)
    }

    /// Tailscale adds keys between releases. A decoder that read an absent key
    /// as a failure would let a Tailscale upgrade break enrollment on a Mac
    /// that never asked for any of this.
    @Test func aDocumentMissingEverythingOptionalStillReads() throws {
        let minimal = """
            {"BackendState": "Running",
             "Self": {"HostName": "Cosmo", "DNSName": "cosmo.tail23af.ts.net."}}
            """
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(minimal)))

        #expect(tailnet.peers.isEmpty)
        #expect(!tailnet.magicDNSEnabled)
        #expect(tailnet.selfNode?.addresses.isEmpty == true)
        // No MagicDNS and no address is a node with nothing to offer, which is
        // not the same as a node that was not there.
        #expect(tailnet.selfAddress == nil)
    }

    // MARK: - Which address gets handed out

    /// The tailnet this was written against has MagicDNS off, so every
    /// `.ts.net` name in its status output resolves for nobody.
    @Test func magicDNSOffMeansTheHundredDotAddress() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))

        #expect(tailnet.selfAddress == "100.66.207.119")
        #expect(tailnet.address(forHostName: "parasky1", orAddress: "") == "100.100.233.51")
    }

    @Test func magicDNSOnMeansTheNameSomebodyWouldRecognize() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.magicDNS)))

        #expect(tailnet.selfAddress == "cosmo.tail23af.ts.net")
        #expect(
            tailnet.address(forHostName: "parasky1", orAddress: "") == "parasky1.tail23af.ts.net")
    }

    // MARK: - Which node a runner is

    /// An address is unique on a tailnet and a name is not, so an address is
    /// believed first.
    @Test func aTailnetAddressIdentifiesItsNodeExactly() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))

        #expect(tailnet.address(forHostName: "", orAddress: "100.97.82.54") == "100.97.82.54")
        #expect(
            tailnet.address(forHostName: "", orAddress: "fd7a:115c:a1e0::7f3b:5237")
                == "100.97.82.54")
        #expect(
            tailnet.address(forHostName: "", orAddress: "paralap.tail23af.ts.net.")
                == "100.97.82.54")
    }

    /// `eliang-machine` is `parasky1.tail23af.ts.net` on the wire — the display
    /// name and the DNS label need not resemble each other at all, so both are
    /// tried.
    @Test func aRunnerIsFoundByTheLabelInEitherOfItsNames() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))

        // The DNS label, out of an `ssh_config` alias.
        #expect(tailnet.address(forHostName: "parasky1", orAddress: "") == "100.100.233.51")
        // The DNS label, out of the mDNS address `ssh -G` resolved to.
        #expect(tailnet.address(forHostName: "", orAddress: "calypso.local") == "100.70.36.97")
        // The display name, which no DNS label matches.
        #expect(tailnet.address(forHostName: "eliang-machine", orAddress: "") == "100.100.233.51")
        #expect(tailnet.address(forHostName: "PARASKY1", orAddress: "") == "100.100.233.51")
    }

    @Test func aRunnerThatIsOnNoTailnetIsNotGuessedAt() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))

        #expect(tailnet.address(forHostName: "workstation", orAddress: "192.168.1.9") == nil)
        #expect(tailnet.address(forHostName: "", orAddress: "") == nil)
    }

    /// A node in this Mac's real tailnet reports its `HostName` as `localhost`,
    /// and `localhost` is also what a runner target says when the runner is
    /// this very machine. Matching the two would grant a phone an address for
    /// somebody's iPad.
    @Test func localhostIsNeverAMatch() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))

        #expect(tailnet.address(forHostName: "localhost", orAddress: "127.0.0.1") == nil)
        // The node itself is still reachable by the names that are its own.
        #expect(tailnet.address(forHostName: "fido", orAddress: "") == "100.116.199.36")
    }

    /// Two nodes called `Carl` is not a corner case somebody constructed: it is
    /// what this Mac's tailnet looks like. The DNS label is assigned and
    /// unique, so `carl` still resolves — to the node the tailnet calls `carl`.
    @Test func aSharedDisplayNameIsDecidedByTheDNSLabel() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))

        #expect(tailnet.address(forHostName: "carl", orAddress: "") == "100.103.22.113")
        #expect(tailnet.address(forHostName: "carl-1", orAddress: "") == "100.124.85.105")
    }

    /// And when no DNS label can break the tie, nothing does. The fixture is
    /// the same two nodes with their labels renamed so that only the shared
    /// `HostName` is left to match on.
    @Test func aSharedDisplayNameWithNoLabelToDecideItMatchesNothing() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.twoCarls)))

        #expect(tailnet.address(forHostName: "carl", orAddress: "") == nil)
        #expect(tailnet.address(forHostName: "carl-a", orAddress: "") == "100.124.85.105")
    }
}

/// What a caller building the ceremony actually asks, which is one question per
/// runner: how far does this address go, and is there a better one.
struct RunnerAddressingTests {
    private func runner(id: String, address: String) -> CeremonyRunner {
        CeremonyRunner(
            id: id, label: id, alias: "", address: address, user: "e-liang", port: 22,
            host_key: "SHA256:x", pending: false)
    }

    /// The bug this exists for: a Mac granted over the LAN, whose own name is
    /// the one thing that stops working when the phone leaves the building.
    @Test func thisMacOnItsMdnsNameIsOfferedItsTailnetAddress() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))
        // The empty id is this Mac, which `tailscale status` reports as `Self`
        // and never as a peer.
        let judged = RunnerFacts.addressing(of: runner(id: "", address: "cosmo.local"), in: tailnet)

        #expect(judged.address == "cosmo.local")
        #expect(judged.reach == .thisNetwork)
        #expect(judged.betterAddress == "100.66.207.119")
    }

    @Test func anotherRunnerIsMatchedThroughItsTarget() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))
        let judged = RunnerFacts.addressing(
            of: runner(id: "deploy@parasky1", address: "192.168.1.44"), in: tailnet)

        #expect(judged.reach == .thisNetwork)
        #expect(judged.betterAddress == "100.100.233.51")
    }

    @Test func anAddressThatAlreadyTravelsIsLeftAlone() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))

        for address in ["100.97.82.54", "box.example.com", "paralap.tail23af.ts.net"] {
            let judged = RunnerFacts.addressing(
                of: runner(id: "box", address: address), in: tailnet)
            #expect(judged.reach == .anywhere, "\(address)")
            #expect(judged.betterAddress == nil, "\(address)")
        }
    }

    /// The ordinary Mac: no Tailscale at all. The reach is still known, which
    /// is the half that costs nothing.
    @Test func noTailscaleIsNotAFailure() {
        let judged = RunnerFacts.addressing(of: runner(id: "", address: "cosmo.local"), in: nil)

        #expect(judged.reach == .thisNetwork)
        #expect(judged.betterAddress == nil)
    }

    /// A runner this Mac reaches over the LAN that is on no tailnet: there is
    /// a warning to give and nothing to suggest.
    @Test func aRunnerWithNoTailnetNodeGetsNoSuggestion() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))
        let judged = RunnerFacts.addressing(
            of: runner(id: "you@workstation", address: "192.168.1.9"), in: tailnet)

        #expect(judged.reach == .thisNetwork)
        #expect(judged.betterAddress == nil)
    }

    /// `ssh -G` said nothing. There is no sentence to write about the address,
    /// but the tailnet still knows the runner by its target.
    @Test func anUnknownAddressStillGetsASuggestionWhenThereIsOne() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.tailnet)))
        let judged = RunnerFacts.addressing(of: runner(id: "calypso", address: ""), in: tailnet)

        #expect(judged.reach == .unknown)
        #expect(judged.betterAddress == "100.70.36.97")
    }

    /// The same Mac on a tailnet that serves its own names, which is the
    /// address a person would recognize as theirs.
    @Test func aTailnetWithMagicDNSOffersTheNameRatherThanTheNumber() throws {
        let tailnet = try #require(Tailnet(json: TailscaleFixture.data(TailscaleFixture.magicDNS)))
        let judged = RunnerFacts.addressing(of: runner(id: "", address: "cosmo.local"), in: tailnet)

        #expect(judged.betterAddress == "cosmo.tail23af.ts.net")
    }
}

/// One `tailscale status --json` document, trimmed out of a real one.
///
/// Every value here is as the daemon printed it, including the things that make
/// the matching awkward: a `HostName` of `localhost`, two nodes called `Carl`,
/// a `DNSName` (`parasky1`) that does not resemble its `HostName`
/// (`eliang-machine`), a two-label `hello.ts.net`, and `MagicDNSEnabled` false.
/// The node keys are shortened and every key this file does not read is gone.
enum TailscaleFixture {
    static func data(_ json: String) -> Data { Data(json.utf8) }

    static let tailnet = """
        {
          "Version": "1.102.1-t8ebe8f7c3-gda6192991",
          "BackendState": "Running",
          "TailscaleIPs": ["100.66.207.119", "fd7a:115c:a1e0::f3b:cf77"],
          "Self": {
            "ID": "nQ6EBm39v121CNTRL",
            "HostName": "Cosmo",
            "DNSName": "cosmo.tail23af.ts.net.",
            "OS": "macOS",
            "TailscaleIPs": ["100.66.207.119", "fd7a:115c:a1e0::f3b:cf77"],
            "Online": true
          },
          "MagicDNSSuffix": "tail23af.ts.net",
          "CurrentTailnet": {
            "Name": "somebody@example.com",
            "MagicDNSSuffix": "tail23af.ts.net",
            "MagicDNSEnabled": false
          },
          "Peer": {
            "nodekey:07be": {
              "HostName": "E-Liang's MacBook Pro",
              "DNSName": "paralap.tail23af.ts.net.",
              "OS": "macOS",
              "TailscaleIPs": ["100.97.82.54", "fd7a:115c:a1e0::7f3b:5237"],
              "Online": true
            },
            "nodekey:2dd5": {
              "HostName": "hello",
              "DNSName": "hello.ts.net.",
              "OS": "linux",
              "TailscaleIPs": ["100.101.102.103", "fd7a:115c:a1e0:ab12:4843:cd96:6265:6667"],
              "Online": true
            },
            "nodekey:3f01": {
              "HostName": "localhost",
              "DNSName": "fido.tail23af.ts.net.",
              "OS": "iOS",
              "TailscaleIPs": ["100.116.199.36", "fd7a:115c:a1e0:ab12:4843:cd96:6274:c724"],
              "Online": false
            },
            "nodekey:4b22": {
              "HostName": "Carl",
              "DNSName": "carl-1.tail23af.ts.net.",
              "OS": "linux",
              "TailscaleIPs": ["100.124.85.105", "fd7a:115c:a1e0::173b:5569"],
              "Online": false
            },
            "nodekey:5c33": {
              "HostName": "Carl",
              "DNSName": "carl.tail23af.ts.net.",
              "OS": "windows",
              "TailscaleIPs": ["100.103.22.113", "fd7a:115c:a1e0:ab12:4843:cd96:6267:1671"],
              "Online": false
            },
            "nodekey:6d44": {
              "HostName": "calypso",
              "DNSName": "calypso.tail23af.ts.net.",
              "OS": "linux",
              "TailscaleIPs": ["100.70.36.97", "fd7a:115c:a1e0:ab12:4843:cd96:6246:2461"],
              "Online": true
            },
            "nodekey:7e55": {
              "HostName": "eliang-machine",
              "DNSName": "parasky1.tail23af.ts.net.",
              "OS": "linux",
              "TailscaleIPs": ["100.100.233.51", "fd7a:115c:a1e0::7f3b:e935"],
              "Online": true
            }
          }
        }
        """

    /// The same tailnet with MagicDNS switched on, which is the only thing that
    /// makes a `.ts.net` name worth handing to another device.
    static let magicDNS = tailnet.replacingOccurrences(
        of: "\"MagicDNSEnabled\": false", with: "\"MagicDNSEnabled\": true")

    /// The same two `Carl` nodes, with their DNS labels renamed so that the
    /// shared `HostName` is the only thing left to match on. Constructed, and
    /// the one fixture here that is: the real tailnet's labels happen to break
    /// their own tie.
    static let twoCarls = tailnet
        .replacingOccurrences(of: "carl-1.tail23af.ts.net.", with: "carl-a.tail23af.ts.net.")
        .replacingOccurrences(of: "carl.tail23af.ts.net.", with: "carl-b.tail23af.ts.net.")
}
