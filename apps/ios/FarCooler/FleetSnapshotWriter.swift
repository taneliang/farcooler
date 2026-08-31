import Foundation
import WidgetKit

/// Turning the fleet the app just polled into the snapshot every other surface
/// renders from.
///
/// Deliberately a projection and not a second model. Each field is copied from
/// what the daemon derived — nothing here decides what a headline says or which
/// agent ranks first, because those are decided on the host precisely so a
/// widget and this app cannot disagree about the same pane.
///
/// Only agent panes reach the snapshot. Every terminal has an `activity` — a
/// plain shell's is the word `none` — so the test is what that word SAYS, not
/// whether the key arrived. A widget listing every terminal on every runner
/// would be a list nobody can find anything in.
enum FleetSnapshotWriter {
    /// `@MainActor` because `WatchLinkHost` is, and because the one caller —
    /// `Connection.refresh` — already is. Nothing here is slow enough to be
    /// worth hopping off it, and hopping would let two polls' snapshots land
    /// out of order.
    @MainActor
    static func write(fleet: Fleet, inbox: [String: InboxRow]?, machine: String) {
        // One instant for the whole poll, and it reaches two places: the
        // snapshot's `capturedAt` and every agent's `observedAt`. They are the
        // same moment here and only here — a poll hears about the entire fleet
        // at once, which is precisely what a push does not do — so reading them
        // off two `Date()` calls a few microseconds apart would be inventing a
        // difference that does not exist.
        let now = Date()
        let agents = fleet.workspaces.flatMap(\.terminals).compactMap { terminal in
            snapshotAgent(terminal, machine: machine, at: now)
        }
        let snapshot = FleetSnapshot(
            agents: agents, capturedAt: now, complete: true,
            reviewsWaiting: reviewsWaiting(inbox),
            // The fleet's rows summed at ONE width, as the runner summed them.
            // Not added up here out of the per-agent traces above, and that is
            // arithmetic rather than deference: each row snapped to the shortest
            // of §04's three windows that held its own activity, so bucket 4 of
            // a five-minute row and bucket 4 of a two-hour row are different
            // spans of time and adding them adds unlike things. The daemon holds
            // every ring and can pick one width across all of them. See
            // `FleetSnapshot.fleetTrace`.
            fleetTrace: fleet.fleetTrace)
        SnapshotStore.write(snapshot)
        // The surfaces are out of process and do not poll. Without this they
        // keep drawing the previous snapshot until the system next decides to
        // refresh them, which can be an hour.
        WidgetCenter.shared.reloadAllTimelines()
        // The same snapshot, to the watch. One projection and two consumers, so
        // a widget on the lock screen and a complication on a wrist cannot
        // disagree about the same pane — which they would the moment either one
        // derived its own. `send` decides whether this is worth a Bluetooth
        // round trip; it is called on every poll precisely so that it, and not
        // this file, holds that rule.
        WatchLinkHost.shared.send(snapshot: snapshot)
    }

    /// How many worktrees are waiting to be looked at, or nil when this
    /// connection has never been told.
    ///
    /// **The rows are already in hand and cost nothing to read here.**
    /// `Connection.refresh` has called `changes.inbox` on every poll since the
    /// phone's review surface landed — it is what draws `+120 -4` beside a row
    /// in `FleetView` — so carrying its answer into the snapshot adds no RPC,
    /// no round trip and no work on the runner. Measured against a live daemon
    /// on this machine, `changes inbox` costs about 0.3 ms over the process
    /// start it shares with `farcooler --version`, and roughly a fortieth of
    /// what `status` costs; on the runner it is `review::cheap_gate`, two
    /// `stat`s per worktree, plus lookups that are already memoized in
    /// `ReviewCache` and a base resolution that is documented never to touch
    /// the network. A second RPC per poll was the cost worth measuring before
    /// building this, and the measurement is that there is not one.
    ///
    /// **Nil when nothing has been read.** An empty dictionary is ambiguous —
    /// it is both "every worktree is reviewed" and "this runner's daemon
    /// predates `changes.inbox` and refuses the call on every poll forever" —
    /// so the caller passes nil for the second, and nil travels all the way to
    /// the surfaces as "not told". See `FleetSnapshot.needsReview`.
    ///
    /// **`changedSinceReviewed` AND a diff to show.** Both halves are needed.
    /// The daemon marks a worktree it has never seen marked read as changed —
    /// `None => true` in `review_ops::inbox` — so counting the flag alone would
    /// greet a freshly set-up runner with a review count equal to its worktree
    /// list, including the ones that changed nothing. Requiring a diff drops
    /// those and leaves the number meaning what the word says: something to
    /// look at, that moved since you looked.
    ///
    /// That cost an UNDERCOUNT for as long as `insertions`/`deletions` were
    /// committed-only: a worktree whose only work was uncommitted reported
    /// `0/0` and dropped out of this count even though the daemon's `touched_at`
    /// signal knew it had moved. `7927c13` fixed it where it belonged, in the
    /// daemon — `change_set::shortstat` now compares the base against the
    /// working tree and counts untracked lines — so an agent that has edited
    /// and not committed reaches this count, with no change here.
    private static func reviewsWaiting(_ inbox: [String: InboxRow]?) -> Int? {
        guard let inbox else { return nil }
        return inbox.values.filter { $0.changedSinceReviewed && $0.hasDiff }.count
    }

    private static func snapshotAgent(
        _ terminal: Terminal, machine: String, at now: Date
    ) -> FleetSnapshot.Agent? {
        // The host's own word for it. `activity` is `none` for a plain shell and
        // for any pane whose process has exited, and something else only where
        // the daemon has classified a coding agent — see `activity_label` in
        // `crates/client/src/session.rs`.
        //
        // The test used to be that the key was present and non-empty, which
        // filtered NOTHING: the daemon always sends it, so a shell arrived as
        // the non-empty string "none" and went into the ranking beside the
        // agents. `farcooler_core::feed::tier` puts a running shell in the same
        // tier as a working agent and a failed `cargo test` in Done's, so a
        // lock screen accessory could lead with `✗ cargo test failed` as the
        // fleet's top agent.
        //
        // Not `isAgentPane`, which asks whether the pane is being RENDERED as a
        // chat. A `claude` started in an ordinary terminal pane is an agent that
        // reports no `paneMode` of `agent` at all, and judging by the render
        // mode would empty these surfaces of exactly the agents most people run.
        // `activity` is the classification; `paneMode` is a view.
        //
        // The second half of the guard is the compiler's, not the rule's:
        // `isAgent` is already false for a nil `activity`. The raw string is
        // carried on rather than `agent.rawValue`, so a status a newer daemon
        // invents survives the trip instead of being folded to "unknown" here.
        guard terminal.agent.isAgent, let activity = terminal.activity else { return nil }
        return FleetSnapshot.Agent(
            id: terminal.id,
            label: terminal.label,
            machine: machine,
            status: activity,
            glyph: terminal.glyph ?? "",
            headline: terminal.headline ?? "",
            line: terminal.signalLine,
            feed: terminal.feed ?? [],
            // A daemon too old to rank sorts last rather than first: it cannot
            // tell us this pane is urgent, and guessing that it is would put an
            // unknown above a known blocked agent.
            rank: terminal.sortRank,
            turnFailed: terminal.turnFailed ?? false,
            // When this state began, as the HOST timed it. This is what stops a
            // push about one agent from re-vouching for the other five: without
            // it every agent's age is the snapshot's own `capturedAt`, which
            // `merging` moves to now. Nil from a daemon too old to send it, and
            // nil then falls back to `capturedAt` exactly as before.
            activityChangedAt: terminal.activityChangedAt,
            // When this phone last heard about this agent, which is now: the
            // daemon just listed it in a fleet poll. The one field on
            // `FleetSnapshot.Agent` that is NOT copied from what the host
            // derived, and deliberately so — the host cannot know when a phone
            // last managed to reach it. See `FleetSnapshot.observedAt`.
            //
            // Every agent in the poll, unconditionally, whether or not anything
            // about it changed: a daemon that says an agent is still working
            // has been heard from, and that is the whole question
            // `confidence(in:at:)` asks. Withholding it for an unchanged agent
            // would put back exactly the bug this fixes — an agent that has
            // been working for an hour reading as "last seen working" on a
            // fleet that is polling every three seconds.
            observedAt: now,
            // How far the agent is through its own task list, straight across
            // and unchanged. Not defaulted the way `feed` and `turnFailed`
            // above are, and the difference is the point: an empty feed and a
            // turn nobody called failed are honest substitutes for silence,
            // whereas `0` of `0` would be a progress claim no host made. A
            // daemon too old to send these — or a codex or cursor pane, whose
            // logs record nothing task-shaped — leaves both nil, and every
            // surface downstream draws nothing rather than an empty bar.
            planDone: terminal.planDone,
            planTotal: terminal.planTotal,
            // §04's thirteen buckets, straight across as the wire's bytes and
            // not decoded on the way. Decoding here would produce three Swift
            // arrays per agent inside a value the widget extension holds once
            // per timeline entry, which is exactly the cost the bytes encoding
            // was chosen to avoid — `proto/farcooler.proto` does the arithmetic
            // at the field. `ActivityTrace` reads them where the drawing is.
            //
            // Nil for a terminal with nothing to show, which is what the
            // producer sends rather than sixty-six zeroes, and nil today for
            // every terminal on every runner: `Session::fleet` does not project
            // this field into the JSON the phone decodes. See
            // `Terminal.activityTrace`.
            trace: terminal.activityTrace)
    }
}
