import Foundation

// Which runners a "Stop Waiting" tap stops.
//
// The rule behind the one button on the screen the app draws before any runner
// has answered. Here for `FleetMembership.swift`'s reason exactly: it is a
// decision with no view in it, and `FleetView` lives in the iOS target, which
// CI compiles and never executes.
//
// **The multi-runner port turned one runner's escape hatch into a fleet-wide
// one and left it there.** Before the port this was one connection's, offered
// only while that connection was `connecting`. After it the body was
// `for runner in fleet.runners { runner.connection.giveUp(...) }` — every
// runner, in every phase, on a screen that appears whenever no runner has a
// fleet yet. Three of those phases cost something real:
//
// - A runner sitting on `needsApproval` has a fingerprint on screen and a
//   person reading it. `giveUp` replaces that with a failure, so the question
//   is gone and the way back to it is two more taps. That question is the
//   single flow onboarding cannot afford to lose.
// - A runner that is `connected` but has not returned its first `fleet` call is
//   the exact state this screen is drawn FOR — `phase` flips a whole SSH round
//   trip before the first fleet arrives. Stopping it kills the poller on a
//   session that is working.
// - A runner already on a failure has a diagnosis, and its message is the only
//   one that exists — "Nothing answered on port 22", "hasn't been given this
//   device's key". `giveUp` overwrites it with "Stopped waiting", which is the
//   app forgetting what it knew.
//
// So this file answers two questions and the screen answers neither: which
// runners a tap stops, and whether to draw the button at all.
enum StopWaiting {
    /// What one runner is doing, as far as "are we still waiting on it" goes.
    ///
    /// Five, and they are `Connection.Phase`'s five under names that say what
    /// the WAIT is rather than what the connection is. A separate vocabulary
    /// because `Phase` is declared in the iOS target and this package cannot see
    /// it — the same reason `RunnerTrouble.Words` takes three strings — and
    /// because the distinction that matters here is not the one `Phase` draws:
    /// what this rule cares about is who is being waited ON.
    enum Standing: Sendable, Equatable, Hashable, CaseIterable {
        /// Dialing, and has never answered. `Phase.connecting`.
        case dialing
        /// Answered once, went away, dialing again. `Phase.reconnecting`.
        case reconnecting
        /// A fingerprint is on screen and a person is being waited on.
        /// `Phase.needsApproval`.
        case asking
        /// Already off the spinner, with a reason. `Phase.failed`.
        case stopped
        /// The session is up. `Phase.connected` — which is NOT the same as
        /// having a fleet, and on this screen it usually is not.
        case answering

        /// Whether the thing being waited on is the network.
        ///
        /// The whole rule, in one place. "Stop waiting" is an answer to a wait
        /// with no end in sight — an address that routes nowhere takes over a
        /// minute to fail on its own — and it is an answer to nothing else. A
        /// person, a working session and a finished failure are each waiting on
        /// something that is not a timeout, and a button labeled for the
        /// timeout must not touch them.
        var isWaitingOnTheNetwork: Bool {
            switch self {
            case .dialing, .reconnecting: return true
            case .asking, .stopped, .answering: return false
            }
        }
    }

    /// The runners a tap actually stops.
    ///
    /// Everything else on the screen is left exactly as it was. Returns the
    /// runners rather than a count so the caller cannot re-derive the set from
    /// a different list than the one it drew — which is the shape the bug had:
    /// the rows iterated the unanswered runners and the button iterated all of
    /// them.
    static func stopping<Runner>(_ runners: [Runner], standing: (Runner) -> Standing)
        -> [Runner]
    {
        runners.filter { standing($0).isWaitingOnTheNetwork }
    }

    /// Whether to draw the button at all.
    ///
    /// False when nothing on screen is stoppable — a lone runner holding a
    /// fingerprint question, most of all. A labeled button that runs and
    /// changes nothing is the same defect as the "Show the Key Again" that
    /// forgot a key nobody had pinned, and it reads worse here: somebody who
    /// taps "Stop Waiting" and keeps waiting has been told the app is stuck.
    static func isOffered<Runner>(_ runners: [Runner], standing: (Runner) -> Standing) -> Bool {
        runners.contains { standing($0).isWaitingOnTheNetwork }
    }
}
