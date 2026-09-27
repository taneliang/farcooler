import Foundation

// What the shell draws before anybody has swiped it, and what an empty grid
// says.
//
// Here rather than in `ShellScreen.body` for this package's usual reason: the
// iOS target has no unit test bundle, so a branch chosen inside a `View` is a
// branch nothing can check. This one had been wrong since the multi-runner
// port and looked exactly like a slow network.
//
// **The defect.** `ShellScreen` opened on a pane and drew a bare `ProgressView`
// until it had one, seeded once off `Connection.hasFleet` — which is set by the
// first successful `refresh` REGARDLESS of what that refresh returned, and
// never cleared. A runner with no worktrees answers, `hasFleet` flips, the one
// seeding pass finds nothing to open on, and `hasFleet` never changes again:
// the spinner is permanent, and creating a worktree later does not re-seed it.
// It is a full-bleed spinner with no navigation bar and no host switcher, so
// there is no way to reach settings, add a runner, or read this device's key
// from it either — the "room with no doors" `FleetView.escapable` exists to
// close, arrived at one screen further in.
//
// A runner with zero worktrees is an ordinary daemon state — a machine set up
// and not yet used is the first thing anybody sees — and the overview has had
// copy for it all along, unreachable because the shell it lives in never got
// mounted.

/// What the shell has to draw before a gesture has moved it.
enum ShellOpening: Hashable {
    /// A pane, and where it is. The ordinary answer.
    case pane(ShellPosition)
    /// Somebody is still on their way, so there may yet be a pane. The only
    /// state that is a spinner, and it now has an end.
    case waiting
    /// Every runner that is going to answer has, and not one of them has a
    /// worktree. A sentence and the ways out of it — never a spinner, because
    /// nothing is being waited for.
    case noWorkspaces
}

enum ShellBringUp {
    /// What one runner has contributed to that decision.
    ///
    /// A vocabulary of its own for `StopWaiting.Standing`'s reason: this
    /// package cannot see `Connection.Phase`, and the distinction that matters
    /// here is not the one `Phase` draws. What this rule asks is only whether
    /// a spinner has an end.
    enum Report: Hashable, CaseIterable {
        /// A `fleet` call from this runner has come back. However many
        /// worktrees it named — including none — they are in the merge.
        case answered
        /// On its way: dialing, reconnecting, or connected with the first
        /// `fleet` call still in flight. `Connection.phase` flips a whole SSH
        /// round trip before that call returns, so "connected" is not
        /// "answered" and the gap is the ordinary occupant of this screen.
        case pending
        /// Not on its way. It failed with a diagnosis, or it is holding a
        /// fingerprint question that only a person can settle — and a person
        /// cannot settle it from behind a spinner. Waiting on either is waiting
        /// forever, which is what the old branch did.
        case stalled
    }

    /// Where the shell opens, or what it says instead.
    ///
    /// - `seated`: the position the shell has already been seeded onto, once
    ///   and only once, or nil before it has been. Sticky, and that is what
    ///   makes this stable: a fleet that empties under a mounted shell does not
    ///   tear the shell down and throw away every pane in it, it leaves an
    ///   overview saying the same sentence.
    /// - `workspaces`: how many the merge has right now. Non-zero with nothing
    ///   seated is the single body pass between a fleet arriving and the
    ///   seeding that runs off it, and it is a wait with an end.
    /// - `reports`: one per runner being talked to.
    static func opening(
        seated: ShellPosition?, workspaces: Int, reports: [Report]
    ) -> ShellOpening {
        if let seated { return .pane(seated) }
        if workspaces > 0 { return .waiting }
        return reports.contains(.pending) ? .waiting : .noWorkspaces
    }
}

/// Whether a tapped card on another runner is still worth landing on.
///
/// With "Connect every runner at once" off, a cached card's tap picks its
/// runner and asks the shell to land on its worktree once that runner answers
/// (`ShellScreen.select(runner:landingOn:)`). The request has to end somehow
/// other than by landing, or the shell jumps to a worktree long after anybody
/// tapped anything. Here rather than in the view for this file's reason.
enum ShellCrossingRule {
    /// Keep waiting, or give the crossing up.
    ///
    /// - `picked`: the runner the crossing names is still the selected one.
    ///   Somebody who picked another runner since has left this one.
    /// - `report`: what that runner has said, or nil while the store has not
    ///   brought it up yet — the turn after the tap, which is not a reason to
    ///   give up.
    /// - `hasWorktree`: whether the runner's own fleet names the worktree,
    ///   read off the same connection as `report` so the two agree.
    ///
    /// Given up when the runner is `.stalled` (failed, or holding a
    /// fingerprint question): kept, it would fire whenever the runner next
    /// answered — after a Retry, a network change or a trust — and close the
    /// grid onto a worktree nobody had asked for since. And given up when the
    /// runner answered without the worktree: it was removed while the card was
    /// a memory.
    static func keeps(picked: Bool, report: ShellBringUp.Report?, hasWorktree: Bool) -> Bool {
        guard picked else { return false }
        switch report {
        case nil, .pending?: return true
        case .stalled?: return false
        case .answered?: return hasWorktree
        }
    }
}

/// What a fleet with no cards in it says.
///
/// One copy, two screens. The overview has said this since it was written and
/// the bring-up screen says it now — and a second transcription of the same
/// three strings is two sentences that drift, which on this one they would:
/// the interesting half is that there are TWO of them and which is right
/// depends on whether anything was typed.
enum ShellEmptyCopy {
    static let title = "No Workspaces"
    static let symbol = "rectangle.on.rectangle.slash"

    /// Nothing matched, versus nothing to match — and they are not the same
    /// sentence. The hand-built empty state quoted the search either way, so a
    /// runner with no worktrees at all was told that none of them matched the
    /// empty string.
    static func description(matching search: String) -> String {
        search.isEmpty
            ? "This runner has no workspaces yet."
            : "No workspace matches “\(search)”."
    }
}
