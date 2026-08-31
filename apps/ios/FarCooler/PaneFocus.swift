import Foundation

/// Which tab of a workspace somebody means.
///
/// This was `Route.Focus`, a case inside the navigation enum `FleetView`
/// rendered as a `NavigationStack` path. That enum is gone — the shell is the
/// app's navigation now, and a shell position is a pair of indices into a
/// fleet, not a list of pushes — but the VALUE outlived it, because it was
/// never really about navigation. It is the one thing this app writes down
/// about a workspace: which of its tabs a person chose.
///
/// A shell-neutral value type in a file of its own, so `Connection` can hold
/// `lastFocus` without importing a vocabulary of screens. Nothing here knows
/// what a shell, a stack or a pane host is.
///
/// Ids and not values, for the reason the route gave and which has not changed:
/// a `Terminal` is a snapshot of what the daemon said a moment ago, and
/// remembering one would be remembering a description of a world that has since
/// moved on. An id either still names something on this runner or it does not,
/// and the second answer is one `ShellFleetMap.resume` can act on.
///
/// ## The order of authority, now that there are two answers rather than three
///
/// The route was the third: a door that named the pane it meant. Every door in
/// the shell is a gesture — a swipe along the bar, a carried lift, a tap on an
/// overview card — and none of them names a tab, because none of them is about
/// a tab. So what is left is:
///
/// 1. **The tab you were last on.** `Connection.lastFocus`, written only when a
///    person moves between the tabs of one workspace — see
///    `Connection.rememberFocus` and `ShellScreen.remember(_:leaving:)`.
/// 2. **The rule**, `rule(for:inbox:)`: whatever needs you now.
///
/// The difference between them is the difference between a place you chose and
/// a place the app picked for you. A workspace you last read the diff of should
/// open on that diff when you come back to it, at the gym ninety seconds later
/// or in the morning — `docs/jobs-to-be-done.md` F4 is the owner saying review
/// has to be resumable, and landing somewhere other than where you were is
/// exactly what it is not to be. A workspace you have never chosen a tab in has
/// nothing to be resumed to, and opens on whatever needs you at seven rather
/// than on the agent that needed you at midnight.
///
/// Both degrade the same way: a remembered or ruled agent that has left the
/// fleet falls through to the next answer rather than to a placeholder. See
/// `ShellFleetMap.resume`, which is the one place the two are read in order.
enum PaneFocus: Hashable, Codable {
    /// One agent, by terminal id.
    case agent(String)
    /// The worktree's own diff.
    case changes
    /// No opinion — see the order above.
    case none
}

extension PaneFocus {
    /// Which tab a workspace opens on when nobody ever chose one.
    ///
    /// The last of the two answers above, and the only one that reads the world
    /// as it is right now rather than as somebody left it.
    ///
    /// Blocked agent, then unread diff, then whatever the fleet would put at the
    /// top. In that order because it is the order of "what did you open this
    /// for": an agent that stopped to ask is waiting on you, a diff that moved
    /// is waiting to be read, and a workspace where neither is true is one you
    /// went looking for rather than one that called.
    ///
    /// The blocked agent is chosen by `sortRank` — `farcooler_core::feed::rank`,
    /// computed on the runner — with the terminal id as a tiebreak, because
    /// ranks genuinely collide and `min(by:)` over a collision has to land on
    /// the same agent every time or the workspace opens somewhere different on
    /// each poll.
    ///
    /// One caller reaches it now, and there used to be three. The inbox row and
    /// the workspace list row were two of them, and both were doors that named a
    /// pane; neither screen exists. What is left is
    /// `ShellFleetMap.resume(_:connection:tabs:)` — a workspace nobody has ever
    /// chosen a tab in, or one whose remembered tab names an agent that has
    /// since gone.
    static func rule(for workspace: Workspace, inbox: InboxRow?) -> PaneFocus {
        // A `changes` pane the host happens to have open is not an agent and is
        // not a candidate: the diff it shows is the Changes tab, which is
        // already the second branch below.
        let panes = workspace.terminals.filter { !$0.isChangesPane }

        if let blocked = panes.filter({ $0.agent == .blocked })
            .min(by: { ($0.sortRank, $0.id) < ($1.sortRank, $1.id) })
        {
            return .agent(blocked.id)
        }
        // Both halves, and they are not the same fact. `hasDiff` is true of
        // every worktree with work on it and stays true after you have read it;
        // `changedSinceReviewed` is the daemon's watermark, and it is what makes
        // this "there is something new here" rather than "there is a branch
        // here". `ShellFleetMap.diffMark` gates the Diff tab's cyan ring on the
        // same pair, so the mark and the landing cannot disagree.
        if let inbox, inbox.changedSinceReviewed, inbox.hasDiff { return .changes }
        if let top = panes.min(by: { ($0.sortRank, $0.id) < ($1.sortRank, $1.id) }) {
            return .agent(top.id)
        }
        // A workspace with no panes at all still has a diff to read, and that
        // is the whole reason Changes needs no pane to exist behind it.
        return .changes
    }
}
