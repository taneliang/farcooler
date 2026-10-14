import Foundation
import Testing

@testable import Far_Cooler

/// Which reach a ceremony reply carries: the tunnel first, the address kept
/// beside it (owner, Oct 13, ov-485).
///
/// **The rule.** A runner that answered with a token is granted through the
/// tunnel, whatever its address, and the address travels as the alternative
/// the new device offers when the tunnel fails. A runner with no token keeps
/// its address. Nothing here dials either: the reply is what the new device
/// writes down.
struct TunnelReachTests {
    /// The token an actual daemon served in
    /// `.claude/agent/reports/end-to-end-on-device.md`, shortened. Only its
    /// emptiness is ever inspected; nothing here has an opinion about its shape.
    static let token = "tco2FwWCC__e_72i80rU8Feeyq"

    // MARK: - The decision

    /// A LAN-only runner with a token goes through the tunnel.
    @Test func aRunnerWhoseAddressStopsAtThisNetworkIsGrantedTheTunnel() {
        let decided = Enrollment.reach(granting: runner(host: "cosmo.local"), token: Self.token)

        #expect(decided == .tailcat(token: Self.token))
    }

    /// **The rule that changed.** A runner whose address works anywhere is
    /// granted the tunnel too: tunnel first, and the address only on request.
    @Test func aRunnerThatTravelsIsGrantedTheTunnelToo() {
        let decided = Enrollment.reach(granting: runner(host: "box.example.com"), token: Self.token)

        #expect(decided == .tailcat(token: Self.token))
    }

    /// No token, no tunnel. A runner serving none, one too old to have one,
    /// and one that could not start one all pair at their address.
    @Test func aRunnerWithNoTokenKeepsItsAddress() {
        let stranded = runner(host: "cosmo.local")

        #expect(Enrollment.reach(granting: stranded, token: "") == stranded.reach)
    }

    /// A runner that already IS a tunnel comes back untouched, carrying its own
    /// token rather than whichever token this enrollment happened to read.
    @Test func aTunneledRunnerKeepsItsOwnToken() {
        let already = CeremonyRunner(
            id: "tunneled", label: "tunneled", alias: "", user: "e-liang",
            host_key: "SHA256:whatever", reach: .tailcat(token: "its-own-token"), pending: true)

        #expect(Enrollment.reach(granting: already, token: Self.token) == .tailcat(token: "its-own-token"))
    }

    // MARK: - The reply the ceremony actually builds

    /// End to end through ``Enrollment/enroll(keyA:keyB:label:clientID:scope:nodeKey:on:using:)``:
    /// every runner that answered with a token is granted the tunnel, and its
    /// address moves beside it as the alternative, never into `reach`.
    @Test func everyRunnerWithATokenIsGrantedTheTunnelWithItsAddressBesideIt() async {
        let runners = [runner(id: "e-liang@cosmo", host: "cosmo.local"),
                       runner(id: "e-liang@box", host: "box.example.com")]

        let outcome = await Enrollment.enroll(
            keyA: EnrollmentTests.keyA, keyB: nil, label: "iPhone 17", clientID: "farcooler-1",
            scope: "control", nodeKey: EnrollmentTests.nodeKey, on: runners,
            using: { _ in (true, #"{"alreadyEnrolled":false,"connBlob":"\#(Self.token)"}"#) })

        let granted = outcome.granting(runners)
        for (runner, host) in zip(granted, ["cosmo.local", "box.example.com"]) {
            #expect(runner.reach == .tailcat(token: Self.token), "\(runner.id) kept its address")
            #expect(runner.alternative == .direct(host: host, port: 22), "\(runner.id) lost its address")
        }
        #expect(outcome.tunneled.keys.sorted() == ["e-liang@box", "e-liang@cosmo"])
    }

    /// A pairing where nothing joined a tunnel is the ordinary pairing, and it
    /// must be untouched: every address kept, no alternative, nothing said.
    ///
    /// This is the constraint that a tunnel failure never fails a pairing,
    /// asserted from the app's side.
    @Test func aPairingWithNoTunnelAnywhereIsTheOrdinaryPairing() async {
        let runners = [runner(id: "e-liang@cosmo", host: "cosmo.local")]

        let outcome = await Enrollment.enroll(
            keyA: EnrollmentTests.keyA, keyB: nil, label: "iPhone 17", clientID: "farcooler-1",
            scope: "control", nodeKey: EnrollmentTests.nodeKey, on: runners,
            using: { _ in (true, #"{"alreadyEnrolled":false,"connBlob":""}"#) })

        let granted = outcome.granting(runners)
        #expect(granted.allSatisfy { !$0.pending })
        #expect(granted.first?.reach == .direct(host: "cosmo.local", port: 22))
        #expect(granted.first?.alternative == nil)
        #expect(outcome.tunneled.isEmpty)
    }

    /// A runner Key A never landed on is pending, and it is not tunneled either:
    /// nothing was written, so nothing was admitted to anything.
    @Test func aRunnerThatRefusedTheKeyIsNeitherWrittenNorTunneled() async {
        let runners = [runner(id: "e-liang@cosmo", host: "cosmo.local")]

        let outcome = await Enrollment.enroll(
            keyA: EnrollmentTests.keyA, keyB: nil, label: "iPhone 17", clientID: "farcooler-1",
            scope: "control", nodeKey: EnrollmentTests.nodeKey, on: runners,
            using: { _ in (false, "Could not reach e-liang@cosmo over ssh.") })

        #expect(outcome.tunneled.isEmpty)
        #expect(outcome.granting(runners).allSatisfy { $0.pending })
    }

    /// The reply decodes with the alternative where the core put it, and an
    /// older reply with none decodes with nil.
    @Test func theAlternativeRoundTripsOnTheWire() throws {
        var granted = runner(host: "cosmo.local")
        granted.alternative = granted.reach
        granted.reach = .tailcat(token: Self.token)
        let data = try JSONEncoder().encode(granted)
        let text = String(decoding: data, as: UTF8.self)
        #expect(text.contains(#""alternative":{"#), "\(text)")
        #expect(try JSONDecoder().decode(CeremonyRunner.self, from: data) == granted)

        let older = try JSONEncoder().encode(runner(host: "cosmo.local"))
        #expect(!String(decoding: older, as: UTF8.self).contains("alternative"))
        #expect(try JSONDecoder().decode(CeremonyRunner.self, from: older).alternative == nil)
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
}
