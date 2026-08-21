import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Deciding whether to tell somebody their runner is out of date.
///
/// This has teeth in both directions, and both failures are invisible by
/// looking. Too shy and a feature the app just shipped silently does nothing on
/// a runner still serving the old build — which is exactly what happened, for
/// fourteen commits, and was only found by reading a binary's build stamp. Too
/// eager and the sidebar carries a permanent badge about a runner that is
/// perfectly fine, next to the runner health it must not be confused with.
///
/// The rule reads four things and only four, which is what makes it testable at
/// all: the link state, the build the daemon reported, whether that read failed
/// rather than merely not having landed, and whether this is a remote runner.
struct DaemonSkewTests {
    private func build(matches: Bool, version: String = "0.1.0+abc1234") -> DaemonBuild {
        DaemonBuild(version: version, matches: matches, platform: "macos")
    }

    @Test func aRunnerBuiltFromThisSourceSaysNothing() {
        let skew = DaemonClient.skew(
            state: .connected, build: build(matches: true), unreadable: false, remote: true)
        #expect(skew == .current)
        #expect(!skew.offersUpdate)
    }

    @Test func aRunnerBuiltFromOlderSourceOffersAnUpdateAndNamesTheBuild() {
        let skew = DaemonClient.skew(
            state: .connected, build: build(matches: false), unreadable: false, remote: true)
        #expect(skew == .behind(daemon: build(matches: false).readable))
        #expect(skew.offersUpdate)
        #expect(skew.daemonVersion != nil)
    }

    /// The distinction the `unreadable` flag exists for. A read that has not
    /// landed yet and a read that came back empty must not look the same, or
    /// the runner most likely to be stale is drawn as one whose answer is
    /// merely in flight.
    @Test func aReadThatHasNotLandedIsNotAReadThatFailed() {
        #expect(
            DaemonClient.skew(state: .connected, build: nil, unreadable: false, remote: true)
                == .unavailable)
        #expect(
            DaemonClient.skew(state: .connected, build: nil, unreadable: true, remote: true)
                == .unknown)
    }

    /// A daemon too old to answer the question claims nothing and offers
    /// nothing. Its version cannot be compared, so neither "current" nor
    /// "behind" is honest.
    @Test func anUnknownVersionOffersNoUpdate() {
        let skew = DaemonClient.skew(
            state: .connected, build: nil, unreadable: true, remote: true)
        #expect(!skew.offersUpdate)
        #expect(skew.daemonVersion == nil)
    }

    /// Connection trouble outranks version news. Nobody knows what an
    /// unreachable runner is running, and a runner mid-connect is not a runner
    /// with a problem.
    @Test func aRunnerYouCannotReachIsNotARunnerThatIsBehind() {
        for state in [HostState.connecting, .reconnecting(attempt: 2), .notInstalled] {
            #expect(
                DaemonClient.skew(state: state, build: nil, unreadable: true, remote: true)
                    == .unavailable,
                "\(state)")
        }
        #expect(
            DaemonClient.skew(
                state: .unreachable(reason: "connection refused"), build: nil, unreadable: false,
                remote: true) == .unavailable)
    }

    /// The one unreachable that IS a version, told apart by the CLI's own
    /// wording. Retrying provably cannot fix it, so it is the one red dot whose
    /// click should update rather than reconnect.
    @Test func aRefusedHandshakeIsAVersionProblem() {
        let skew = DaemonClient.skew(
            state: .unreachable(reason: "protocol mismatch; update the older side"),
            build: nil, unreadable: false, remote: true)
        #expect(skew == .tooOldToTalk)
        #expect(skew.offersUpdate)
    }

    /// This Mac cannot be a protocol version behind itself, so the same message
    /// arriving for the local runner is an ordinary failure. Guarding on it
    /// keeps a local hiccup from offering an update that would restart the
    /// daemon — and discard every local agent conversation — for nothing.
    @Test func theLocalRunnerIsNeverAVersionBehindItself() {
        #expect(
            DaemonClient.skew(
                state: .unreachable(reason: "protocol mismatch; update the older side"),
                build: nil, unreadable: false, remote: false) == .unavailable)
    }
}
