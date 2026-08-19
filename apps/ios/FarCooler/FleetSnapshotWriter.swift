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
    static func write(fleet: Fleet, machine: String) {
        let agents = fleet.workspaces.flatMap(\.terminals).compactMap { terminal in
            snapshotAgent(terminal, machine: machine)
        }
        let snapshot = FleetSnapshot(agents: agents, capturedAt: Date(), complete: true)
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

    private static func snapshotAgent(
        _ terminal: Terminal, machine: String
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
            activityChangedAt: terminal.activityChangedAt)
    }
}
