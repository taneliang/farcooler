import Foundation
import Testing

@testable import Far_Cooler

/// Which reach a ceremony reply carries, and why it is almost never the tunnel.
///
/// **The rule.** A ``CeremonyRunner`` is an address or a token and never both.
/// The new device writes it down once and dials it forever — no re-discovery on
/// either side, no second attempt at connect time — so the reply must be right
/// the first time, and a wrong answer is a runner that is simply gone.
///
/// **The trap.** `enrollment::tunnel_route` creates a tunnel identity on EVERY
/// pairing that carries a node key, so almost every runner in a tunneled pairing
/// answers with a token. A rule of "a token means offer the tunnel" would move a
/// whole working fleet off its addresses, and it would pass any test that only
/// ever showed it one runner. Half of these tests are about a token that must be
/// ignored, and that is the half worth having.
struct TunnelReachTests {
    /// The token an actual daemon served in
    /// `.claude/agent/reports/end-to-end-on-device.md`, shortened. Only its
    /// emptiness is ever inspected; nothing here has an opinion about its shape.
    static let token = "tco2FwWCC__e_72i80rU8Feeyq"

    // MARK: - What tips it

    /// A LAN-only address with nothing better anywhere, and a runner that
    /// answered with a token. This is the case the tunnel exists for: the new
    /// device's alternative is not a slower route, it is no route.
    @Test func aRunnerWhoseAddressStopsAtThisNetworkIsOfferedTheTunnel() {
        let decided = Enrollment.reach(
            granting: runner(host: "cosmo.local"), token: Self.token,
            addressing: verdict(.thisNetwork, better: nil))

        #expect(decided == .tailcat(token: Self.token))
    }

    /// **The one that would pass by accident.** A directly reachable runner
    /// keeps its address even though it handed back a token, because being
    /// handed a node key is what makes a runner mint an identity — not a
    /// judgement that its address is unusable.
    @Test func aRunnerThatTravelsKeepsItsAddressThoughItHasAToken() {
        let box = runner(host: "box.example.com")

        let decided = Enrollment.reach(
            granting: box, token: Self.token, addressing: verdict(.anywhere, better: nil))

        #expect(decided == box.reach)
    }

    /// And the same for a LAN address this Mac already fixed. `AddDeviceView`
    /// swaps a tailnet address onto the record before anything is written, so
    /// the verdict still says `.thisNetwork` while the runner being granted
    /// travels — reading `reach` alone here would spend a working address.
    @Test func aRunnerGivenATailnetAddressKeepsIt() {
        let swapped = runner(host: "cosmo.tail23af.ts.net")

        let decided = Enrollment.reach(
            granting: swapped, token: Self.token,
            addressing: verdict(.thisNetwork, better: "cosmo.tail23af.ts.net"))

        #expect(decided == swapped.reach)
    }

    /// No token, no tunnel — whatever the address is. A runner serving no
    /// tunnel, one never asked, and one that could not start one all answer the
    /// same way, and all three pair as direct.
    @Test func aRunnerWithNoTokenKeepsItsAddressEvenWhenItGoesNowhere() {
        let stranded = runner(host: "cosmo.local")

        let decided = Enrollment.reach(
            granting: stranded, token: "", addressing: verdict(.thisNetwork, better: nil))

        #expect(decided == stranded.reach)
    }

    /// A runner this Mac formed no opinion about keeps its address. Reading a
    /// missing judgement as "does not travel" would spend a working address on
    /// a guess.
    @Test func noVerdictLeavesTheAddressAlone() {
        let box = runner(host: "192.168.1.180")

        let decided = Enrollment.reach(granting: box, token: Self.token, addressing: nil)

        #expect(decided == box.reach)
    }

    /// `0.0.0.0` and friends: `.unknown` refuses to guess WHY the address is
    /// unusable, which does not make it usable. The tunnel is the better answer.
    @Test func anAddressThatIsNotAnAddressIsOfferedTheTunnel() {
        let decided = Enrollment.reach(
            granting: runner(host: "0.0.0.0"), token: Self.token,
            addressing: verdict(.unknown, better: nil))

        #expect(decided == .tailcat(token: Self.token))
    }

    /// A runner that already IS a tunnel comes back untouched, carrying its own
    /// token rather than whichever token this enrollment happened to read.
    @Test func aTunneledRunnerKeepsItsOwnToken() {
        let already = CeremonyRunner(
            id: "tunneled", label: "tunneled", alias: "", user: "e-liang",
            host_key: "SHA256:whatever", reach: .tailcat(token: "its-own-token"), pending: true)

        let decided = Enrollment.reach(
            granting: already, token: Self.token, addressing: verdict(.anywhere, better: nil))

        #expect(decided == .tailcat(token: "its-own-token"))
    }

    // MARK: - The reply the ceremony actually builds

    /// End to end through ``Enrollment/enroll(keyA:keyB:label:clientID:scope:nodeKey:on:addressing:using:)``:
    /// one fleet, one token each, and only the stranded runner moves.
    @Test func onlyTheStrandedRunnerInAFleetIsGrantedThroughTheTunnel() async {
        let runners = [runner(id: "e-liang@cosmo", host: "cosmo.local"),
                       runner(id: "e-liang@box", host: "box.example.com")]
        let judged = [
            "e-liang@cosmo": verdict(.thisNetwork, better: nil),
            "e-liang@box": verdict(.anywhere, better: nil),
        ]

        let outcome = await Enrollment.enroll(
            keyA: EnrollmentTests.keyA, keyB: nil, label: "iPhone 17", clientID: "farcooler-1",
            scope: "control", nodeKey: EnrollmentTests.nodeKey, on: runners, addressing: judged,
            using: { _ in (true, #"{"alreadyEnrolled":false,"connBlob":"\#(Self.token)"}"#) })

        let granted = outcome.granting(runners)
        #expect(granted.first { $0.id == "e-liang@cosmo" }?.reach == .tailcat(token: Self.token))
        #expect(
            granted.first { $0.id == "e-liang@box" }?.reach
                == .direct(host: "box.example.com", port: 22))
        // And the sentence follows the decision, not the tokens.
        #expect(outcome.tunneled.keys.sorted() == ["e-liang@cosmo"])
    }

    /// A pairing where nothing joined a tunnel is the ordinary pairing, and it
    /// must be untouched: every address kept, and nothing said about tunnels.
    ///
    /// This is the constraint that a tunnel failure never fails a pairing,
    /// asserted from the app's side — the daemon turns every tunnel failure into
    /// an empty token with the pairing still succeeding, and an app that read an
    /// empty token as a failure would undo that.
    @Test func aPairingWithNoTunnelAnywhereIsTheOrdinaryPairing() async {
        let runners = [runner(id: "e-liang@cosmo", host: "cosmo.local")]

        let outcome = await Enrollment.enroll(
            keyA: EnrollmentTests.keyA, keyB: nil, label: "iPhone 17", clientID: "farcooler-1",
            scope: "control", nodeKey: EnrollmentTests.nodeKey, on: runners,
            addressing: ["e-liang@cosmo": verdict(.thisNetwork, better: nil)],
            using: { _ in (true, #"{"alreadyEnrolled":false,"connBlob":""}"#) })

        let granted = outcome.granting(runners)
        #expect(granted.allSatisfy { !$0.pending })
        #expect(granted.first?.reach == .direct(host: "cosmo.local", port: 22))
        #expect(outcome.tunneled.isEmpty)
    }

    /// A runner Key A never landed on is pending, and it is not tunneled either:
    /// nothing was written, so nothing was admitted to anything.
    @Test func aRunnerThatRefusedTheKeyIsNeitherWrittenNorTunneled() async {
        let runners = [runner(id: "e-liang@cosmo", host: "cosmo.local")]

        let outcome = await Enrollment.enroll(
            keyA: EnrollmentTests.keyA, keyB: nil, label: "iPhone 17", clientID: "farcooler-1",
            scope: "control", nodeKey: EnrollmentTests.nodeKey, on: runners,
            addressing: ["e-liang@cosmo": verdict(.thisNetwork, better: nil)],
            using: { _ in (false, "Could not reach e-liang@cosmo over ssh.") })

        #expect(outcome.tunneled.isEmpty)
        #expect(outcome.granting(runners).allSatisfy { $0.pending })
    }

    // MARK: - Reading the token out of what the CLI said

    /// The reply shares its string with whatever the CLI logged, because
    /// `CLI.run` concatenates stdout and stderr. Parsing the join would fail on
    /// every runner that logged anything at all.
    @Test func theTokenIsFoundBesideTheCLIsOwnLogging() {
        let output = """
            2026-09-06T04:22:23Z  INFO farcooler: connecting to e-liang@cosmo
            {"client":{"clientId":"farcooler-1"},"alreadyEnrolled":false,"connBlob":"\(Self.token)"}
            2026-09-06T04:22:24Z  INFO farcooler: done
            """

        #expect(Enrollment.token(in: output) == Self.token)
    }

    /// An older CLI's reply has no such key, and output that is not JSON at all
    /// is what a failure looks like. Both read as "no tunnel" and keep the
    /// address — never as a reason to fail the pairing.
    @Test func aReplyWithNoTokenInItIsNoTunnel() {
        #expect(Enrollment.token(in: #"{"client":null,"alreadyEnrolled":true}"#) == "")
        #expect(Enrollment.token(in: "Could not reach e-liang@cosmo over ssh.") == "")
        #expect(Enrollment.token(in: "") == "")
    }

    /// `already_enrolled` is not a failure — it is the ordinary outcome of
    /// pairing a phone a second time against a runner it is already on, and the
    /// daemon still admits the key and answers with the token.
    @Test func anAlreadyEnrolledDeviceStillGetsItsToken() {
        let output = #"{"client":{"clientId":"farcooler-1"},"alreadyEnrolled":true,"connBlob":"\#(Self.token)"}"#

        #expect(Enrollment.token(in: output) == Self.token)
    }

    // MARK: - Fixtures

    private func runner(id: String = "e-liang@cosmo", host: String) -> CeremonyRunner {
        CeremonyRunner(
            id: id, label: id, alias: "", user: "e-liang", host_key: "SHA256:whatever",
            reach: .direct(host: host, port: 22), pending: true)
    }

    /// One ``RunnerFacts/Addressing``, built the way
    /// ``RunnerFacts/addressing(of:in:)`` builds one: a `betterAddress` only
    /// ever accompanies an address that does not travel.
    private func verdict(_ reach: RunnerFacts.Reach, better: String?) -> RunnerFacts.Addressing {
        RunnerFacts.Addressing(address: "unused", reach: reach, betterAddress: better)
    }
}
