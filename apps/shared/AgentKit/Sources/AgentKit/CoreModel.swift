import Foundation

// The shapes the client core returns — the FFI's, and ONLY the FFI's.
//
// Every type here decodes something `crates/client/src/session.rs` builds with
// `json!` and hands back across the C ABI in `ClientCore.swift`. The phone is
// the only client that reads that producer. The Mac shells out to `farcooler
// … --json` and decodes the CLI's output into its own `Workspace`, `Terminal`
// and `Fleet` in `apps/macos/Sources/FarCooler/Model.swift`, and the two
// producers do not agree: `Session::fleet` sends `isMainCheckout` where the CLI
// sends `is_main_checkout`, `Session::branches` sends `updatedAt` in
// MILLISECONDS where the CLI sends it in seconds, and `repository` is a UUID
// here and a display name there. Same key, different value; same field,
// different spelling.
//
// This file's own header used to say these types were "identical to the Mac
// app's, because both decode what one Rust crate produces — there is one
// definition of what a workspace looks like on the wire, not one per platform".
// That was never true and `07e75e8` is what it cost: `is_main_checkout` was
// copied across from the Mac, decoded to nil for every workspace on every
// phone, and put "Remove Worktree…" on the one worktree it must never be
// offered for. Don't copy a key from the Mac's model. Read the producer this
// app reads.
//
// WHY THESE LIVE IN AgentKit, which the Mac also depends on. Two reasons, and
// the second is what makes the first safe:
//
//  1. So they can be TESTED. The iOS target's only test bundle is a UI-testing
//     one, which cannot `@testable import` the app module, so nothing could
//     reach these while they sat in `apps/ios/FarCooler/`. AgentKit has a real
//     test target that runs on the host with no simulator, and
//     `FleetDecodeTests` now decodes a fixture transcribed key-for-key from
//     `Session::fleet` into these exact types. A field renamed on either end
//     fails a test instead of going quiet on a phone.
//
//  2. Everything in this file is INTERNAL, deliberately, and that is load-
//     bearing rather than incidental. AgentKit vends only its `public` surface
//     to the Mac, so `import AgentKit` over there brings in nothing from here:
//     the Mac cannot see these names, cannot shadow its own with them, and
//     cannot start using them by accident. `302fb73` refused to hoist
//     `reviewAgentTargets()` for exactly this reason — `Workspace` and
//     `Terminal` are declared once per app and the declarations DISAGREE, the
//     phone's `canSwitchPaneMode` being `chatCapable == true` where the Mac's
//     is that AND `!isChangesPane` — and moving these here does not unify them
//     and must not be read as trying to. The access level is the enforcement.
//
// On iOS there is no `import AgentKit` to write: `generate-project.py` compiles
// AgentKit's sources straight into the app module, so this file is part of
// "Far Cooler" exactly as it was when it sat next to `Connection.swift`.

struct Fleet: Decodable {
    var runtimeHealthy: Bool
    var livePanes: Int
    var workspaces: [Workspace]
    /// Every terminal's trace added together, at one width for the whole fleet.
    ///
    /// `TerminalList.fleet_trace` on the proto wire. **Summed on the runner and
    /// never here**, because the rows do not share a width: each snaps to the
    /// shortest window holding its own activity, so bucket 4 of a five-minute
    /// row and bucket 4 of a two-hour row are different spans of time. The
    /// daemon holds every ring and can pick one width across all of them; a
    /// client holding only the rendered rows cannot. See
    /// `FleetSnapshot.fleetTrace`, which carries this to the surfaces.
    ///
    /// Optional and nil-when-absent on the same terms as
    /// `Terminal.activityTrace`, including the last paragraph of its comment:
    /// `Session::fleet` sends no key for this yet either.
    var fleetTrace: Data?

    enum CodingKeys: String, CodingKey {
        case runtimeHealthy = "runtime_healthy"
        case livePanes = "live_panes"
        case workspaces
        case fleetTrace
    }

    static let empty = Fleet(runtimeHealthy: false, livePanes: 0, workspaces: [])
}

struct Workspace: Decodable, Identifiable, Hashable {
    var id: String
    var short: String
    /// Which repository this worktree belongs to, as a UUID STRING — never as
    /// something to show a person.
    ///
    /// One key with two meanings, and this is the half the phone gets.
    /// `Session::fleet` sends `uuid_of(&w.repository_id).to_string()`, which is
    /// what `stack.get` and `pr.refresh` take as their repository argument. The
    /// CLI sends the repository's DISPLAY NAME under this same key, and the
    /// Mac's `Workspace.repository` is therefore a label it puts in a window
    /// subtitle and matches searches against. Both clients are right about the
    /// producer they read, and nothing but this comment says so — which makes
    /// it the same trap `is_main_checkout` was, standing open. Drawing this
    /// string in a row would print a UUID; passing the Mac's to `stack.get`
    /// would ask the daemon about a repository named "overnight".
    ///
    /// Optional because an older daemon's fleet never carried it, and one
    /// missing field must not fail the decode of the whole fleet. Everything
    /// repository-scoped a client can ask about a workspace — its stack, its
    /// pull request — needs this.
    var repository: String?
    var task: String
    var branch: String
    var worktree: String?
    var state: String
    var terminals: [Terminal]

    /// The user asked not to see this one.
    var isHidden: Bool { state == "hidden" }

    /// git no longer lists this worktree, but the row carries terminals.
    var worktreeMissing: Bool { state == "worktree_missing" }

    /// Whether this workspace IS the repository's own checkout — offering to
    /// remove it would offer to delete the directory the repository itself
    /// lives in.
    ///
    /// Named for the wire key exactly, and the spelling is the whole point.
    /// Every property on `Workspace` and `Terminal` is, because a synthesized
    /// decoder matches on the property NAME; the one mapping in this file is
    /// `Fleet`'s, for the only two snake_case keys the FFI emits. This one was
    /// spelled `is_main_checkout` for as long as the phone has had it, which is
    /// the Mac's spelling and the CLI's — `farcooler workspace list --json`
    /// sends snake_case and the Mac decodes that. The phone does not read the
    /// CLI. It reads `Session::fleet` (crates/client/src/session.rs), which
    /// sends `isMainCheckout`, so the key never matched, this was nil for every
    /// workspace, and `isPrimaryCheckout` answered false for all of them — which
    /// put "Remove Worktree…" on the one worktree it must never be offered for.
    /// Don't copy a key across from the Mac's model without checking which
    /// producer this app actually decodes.
    ///
    /// Optional because an older daemon never sent this key, and decoding must
    /// not fail the entire fleet over one absent field.
    var isMainCheckout: Bool?

    /// The decided answer, for the two screens that draw it.
    ///
    /// Absent reads as false, which is the direction that OFFERS the removal —
    /// safe only because the daemon refuses it independently: `remove_worktree`
    /// (crates/daemon/src/service.rs) checks the stored flag and then compares
    /// the worktree path against the repository's own, and both run before it
    /// touches a terminal or a directory. This keeps the button off the menu so
    /// nobody is walked through a destructive confirmation that cannot succeed;
    /// it is not what makes the checkout safe.
    var isPrimaryCheckout: Bool { isMainCheckout ?? false }

    /// Which of several identically-labeled terminals each one is, keyed by
    /// terminal id.
    ///
    /// Ported from the Mac app's `WorkspaceSection.ordinals`. Two `claude`
    /// panes in one workspace are genuinely alike, so they get `1` and `2` —
    /// but only when there is something to tell apart, or a lone `shell`
    /// would be numbered for no reason. Shared by the fleet list, the
    /// terminal screen's title, and its tab strip, so the same terminal is
    /// never numbered differently depending on which screen is showing it.
    func ordinals() -> [String: Int] {
        var counts: [String: Int] = [:]
        for terminal in terminals { counts[terminal.label, default: 0] += 1 }
        var seen: [String: Int] = [:]
        var out: [String: Int] = [:]
        for terminal in terminals where counts[terminal.label, default: 0] > 1 {
            let next = (seen[terminal.label] ?? 0) + 1
            seen[terminal.label] = next
            out[terminal.id] = next
        }
        return out
    }
}

struct Terminal: Decodable, Identifiable, Hashable {
    var id: String
    var short: String
    var title: String
    var preset: String
    var state: String
    /// How the process ENDED: the code it exited with, and the signal that
    /// killed it.
    ///
    /// Both have been on the wire beside `state` all along — see `exitCode` and
    /// `exitSignal` in the fleet JSON, `crates/client/src/session.rs:297-298` — and
    /// both were dropped on the way in here, so an `exited` row on a phone said
    /// only that the process was gone. A shell somebody closed and a `cargo
    /// build` that broke are the same word to `state`, and they are not the
    /// same news. The Mac has read them since terminals learned to report one.
    ///
    /// Optional for the reason every field added to this type is optional: a
    /// daemon built before exit status existed sends no key, and a key this
    /// decoder required would fail the WHOLE fleet rather than cost one row its
    /// ending. Absent means "nobody said", never "it exited cleanly" — see
    /// `runDidFail`, which refuses to read one as the other.
    var exitCode: Int?
    var exitSignal: Int?
    /// What the agent is doing, derived on the HOST. A phone has no screen to
    /// inspect, so this arriving over the wire is the only way it can know —
    /// and it is why the same badge means the same thing here as on the Mac.
    var activity: String?
    /// Whether the turn the agent just finished DIED rather than completed.
    ///
    /// Read from the agent's own session log on the host, and carried beside
    /// `activity` because that has no word for it: a turn that died and one
    /// that succeeded are both `done` there, so the phone drew a green
    /// checkmark for an agent that had stopped working. Absent on older
    /// daemons, and absent means "nothing claimed the turn went badly".
    var turnFailed: Bool?
    /// Unix milliseconds when the current `activity` began, or nil when the host
    /// did not say.
    ///
    /// Distinct from `turnStartedAt`: this restarts whenever the state changes,
    /// so it answers "how long has this been blocked" rather than "how long has
    /// this turn been running". Sent as `activitySince` — see `activity_since`
    /// in `crates/client/src/session.rs` — and timed on the host rather than
    /// here, because a clock started on the phone restarts at every reconnect
    /// and lies across a laptop sleep.
    ///
    /// Optional like every other field added to this type, and for the reason
    /// that rule exists: a daemon built before it sends no key at all, and a key
    /// this decoder required would fail the WHOLE fleet rather than cost one row
    /// its age.
    var activitySince: Double?
    /// Unix milliseconds when the current turn started, or nil between turns.
    ///
    /// Held across Blocked on the host: approving a tool call does not begin a
    /// new turn, so a card's timer does not restart when you answer one.
    var turnStartedAt: Double?
    /// What the agent is asking, while it is asking it.
    var blockedQuestion: String?
    /// The last few things the agent SAID, oldest first, at most three.
    ///
    /// A transcript and only a transcript — the agent's own prose, with no verb
    /// in front of it. What it DID arrives on `line` instead. Already redacted
    /// and cut to a row's width by the daemon, so this app renders them and
    /// decides nothing about them.
    ///
    /// Optional because a daemon from before this existed sends no key, and a
    /// row with no feed must read as "nothing to say" rather than as a decoding
    /// failure that takes the whole fleet down.
    var feed: [String]?
    /// The last thing the agent said, WHOLE and from its opening.
    ///
    /// The same message `feed`'s last lines were cut from, cut from the other
    /// end and to a notification's width rather than a row's — and a separate
    /// field because it cannot be recovered from those lines: a feed entry is
    /// a wrapped ROW, so the last of them is the last forty characters of the
    /// window. That is how a lock screen came to read "batches to avoid N+1
    /// shits." about a turn that had ended "More shit. An industrial quantity
    /// of shit, shipped in carefully authorized batches to avoid N+1 shits."
    ///
    /// Cut on the host to about 120 characters — roughly the two lines a
    /// banner shows, less the workspace name in front of it; see
    /// `farcooler_core::feed::SAID_WIDTH`. This app renders it and decides
    /// nothing about it.
    ///
    /// Optional because a daemon from before this existed sends no key, which
    /// `lastSaid` reads as "ask the feed instead" rather than as nothing said.
    var said: String?
    /// Where the agent is, in one line: the question it is blocked on, its
    /// position in its own task list, or what it is doing right now.
    ///
    /// One rung of the daemon's compact ladder. The priority between those is
    /// decided on the host — see `farcooler_core::feed::line` — because a Mac,
    /// a phone and a watch deciding it separately is three surfaces disagreeing
    /// about one pane.
    var line: String?
    /// The state in one character: `?` blocked, `●` working, `✓` done, `✗`
    /// failed, `·` idle. The narrowest rung, for a lock screen accessory.
    var glyph: String?
    /// The state plus just enough to say whose, at most ~18 characters.
    var headline: String?
    /// Where this terminal sorts in a fleet view. SMALLER sorts FIRST: blocked
    /// outranks done outranks working, and within a tier the oldest first.
    ///
    /// Computed on the host beside `activity`, so a widget showing one agent
    /// and this list showing twelve agree about which one matters.
    var rank: UInt32?
    /// How far the agent is through its OWN task list: `planDone` of 4 and
    /// `planTotal` of 7 is `4/7`.
    ///
    /// The same position `line` may already state in words, carried as the
    /// numbers it was composed from. Separate fields rather than something read
    /// back out of that string, because `line` is a RUNG: the question outranks
    /// the task count, so a blocked agent's line is the question and holds no
    /// numbers at all — and a phone parsing prose the host composed would be a
    /// second derivation of a fact the ladder exists to derive once.
    ///
    /// Optional, like every other field added to this type, and nil is not
    /// zero. Nil is "the host said nothing about a task list", which is a
    /// daemon too old to send these, a pane with no session log, an agent that
    /// never wrote a list, and every codex and cursor pane — their logs record
    /// nothing task-shaped. `0` of `7` is a written list with nothing finished,
    /// which is a different thing and reads differently.
    ///
    /// `planTotal` moves in both directions mid-turn: an agent adds tasks as it
    /// finds work, and a task it deletes counts toward neither half. See
    /// `plan_done` in `proto/farcooler.proto`.
    var planDone: UInt32?
    var planTotal: UInt32?
    /// §04's thirteen buckets, as the wire's 66 bytes. See `ActivityTrace`,
    /// which is the only thing that reads them, and `crates/core/src/trace.rs`,
    /// which is the only thing that writes them.
    ///
    /// `Data` decodes from a base64 string, which is what JSON has for bytes and
    /// what `serde_json` writes for a `Vec<u8>` behind `serde_bytes`. Carried as
    /// bytes rather than as three arrays for the memory reason set out at
    /// `FleetSnapshot.Agent.trace`; nothing between the socket and the drawing
    /// unpacks them.
    ///
    /// **Optional, and nil is not a quiet terminal.** Absent means either "this
    /// terminal has done nothing the trace can see" — the producer sends no
    /// bytes at all in that case, deliberately — or "the daemon on the other end
    /// is too old to have a trace". Both draw as no trace, which is the honest
    /// answer to both. Sixty-six zero bytes would be a third thing and is not
    /// this.
    ///
    /// **`Session::fleet` does not send this key yet.** The daemon puts the
    /// trace on the proto wire — `Terminal.activity_trace`, field 35, and
    /// `crates/daemon/src/rpc.rs` fills it — but the JSON projection the phone
    /// actually decodes (`crates/client/src/session.rs`) has no line for it, so
    /// on today's runner this is nil for every terminal and every Apple surface
    /// draws no trace. That is the correct behaviour for a field nobody sends;
    /// it is not the intended end state. One `"activityTrace": t.activity_trace`
    /// beside `"planDone"` over there lights all of this up.
    var activityTrace: Data?
    /// The agents this agent spawned and has not finished with, named.
    /// Their COUNT is already inside `line`; these are the names.
    var subagents: [String]?
    var epoch: Int
    /// What this terminal's pane is hosting. Absent on older daemons, which is
    /// why it is optional rather than defaulted to something that would look
    /// like a real answer.
    var paneMode: String?
    var chatCapable: Bool?
    var agentSessionId: String?
    var agentMode: String?
    var availableAgentModes: [String]?

    var agent: AgentActivity { AgentActivity.parse(activity) }

    /// Whether the turn this agent just finished, died — and whether saying so
    /// is still the news.
    ///
    /// Gated on `done`, which is the daemon's word for "finished and nobody
    /// has looked yet". The failure belongs to the turn that ENDED: an agent
    /// already working again is not failing, and one whose row has been read
    /// and cleared has been told.
    ///
    /// Deliberately a property beside `agent` rather than a case inside it.
    /// `AgentActivity` is the daemon's own vocabulary — it is what
    /// `terminal.seen` clears, what the notification dedup is keyed on, and
    /// what the task composer waits for — and a fifth value invented on the
    /// client would have to be understood by all of them. Only the two places
    /// that DRAW a state need to know, and they ask for it here.
    var turnDidFail: Bool { agent == .done && turnFailed == true }

    /// Whether the PROCESS ended badly, as opposed to the turn that ran inside
    /// it.
    ///
    /// The companion to `turnDidFail`, and deliberately a separate question:
    /// that one is about the agent's last turn, read from its session log; this
    /// one is about the command, read from how its process exited. A `cargo
    /// build` that returned 101 has no turns at all.
    ///
    /// The Mac's rule, verbatim — see the `.exited` branch of the `Status`
    /// derivation in `apps/macos/Sources/FarCooler/Model.swift:408-416`. A
    /// signal or a non-zero code is a failure worth seeing; a clean exit is
    /// not; and an ABSENT code is not a failure either. That last clause is the
    /// one that matters, because an older daemon sends no exit status at all,
    /// and reading nothing as broken would mark every finished terminal on the
    /// runner as failed.
    ///
    /// Gated on `exited` on the same terms the Mac gates it, so the two apps
    /// cannot disagree about which terminals ended badly: `state` is the
    /// daemon's word for whether the process is gone, and how it ended is a
    /// question only a process that HAS ended can answer.
    var runDidFail: Bool {
        guard StateKind.parse(state) == .exited else { return false }
        return exitSignal != nil || (exitCode.map { $0 != 0 } ?? false)
    }

    /// What this agent's state is called in a row, failure included.
    var activityLabel: String { turnDidFail ? "Failed" : agent.label }

    /// The glyph for it. The same mark the ladder puts on a failed turn and a
    /// failed command, in SF Symbols' vocabulary rather than a character's.
    var activitySymbol: String { turnDidFail ? "xmark.circle.fill" : agent.symbol }

    /// Whether to draw a chat or a VT grid.
    var isAgentPane: Bool { paneMode == "agent" }

    /// Whether this pane is a review of what its worktree changed.
    ///
    /// The daemon has served this mode since the review surface landed, and the
    /// phone had no branch for it: a `changes` pane fell past `isAgentPane` to
    /// the VT renderer and was drawn as a raw terminal — a grid of whatever
    /// bytes were on a pane that is not a tty. See `ChangesView`.
    var isChangesPane: Bool { paneMode == "changes" }

    /// Whether this pane can be shown as a chat.
    ///
    /// Answered on the host, because identifying an agent takes a screen read —
    /// Claude Code renames its own process. Absent from older daemons, and
    /// absent means "do not offer": a switch that came back as a different
    /// agent is worse than no switch at all.
    var canSwitchPaneMode: Bool { chatCapable == true }

    /// The signal line, or empty when the host has nothing to say.
    ///
    /// Trimmed here rather than at each call site: a line that is whitespace is
    /// a line that draws a blank row and makes every surface taller for
    /// nothing, and three surfaces trimming it separately is three chances to
    /// forget.
    var signalLine: String {
        (line ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// When this activity began, as a date, or nil when the host did not say.
    ///
    /// Nil is "not told", which is a different thing from "just now" and must
    /// never be rendered as it — a snapshot that treated an absent timestamp as
    /// the present would vouch for an agent nobody has heard from.
    var activityChangedAt: Date? {
        activitySince.map { Date(timeIntervalSince1970: $0 / 1000) }
    }

    /// The last few things the agent said, trimmed and capped at three.
    ///
    /// Ported from the Mac's `recentSteps` verbatim, including the cap: the
    /// daemon already keeps only three, and repeating the limit here means a
    /// host that ever sent four could not make one row twice the height of
    /// every other row in the list.
    ///
    /// Kept when the agent goes idle rather than cleared. "What did this do
    /// while I was away" is exactly when the summary is worth most, and a row
    /// that shed its lines on going idle would also mean the list rearranging
    /// itself under somebody reading it.
    var recentSteps: [String] {
        let steps = (feed ?? [])
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        return Array(steps.suffix(3))
    }

    /// What to quote in a notification about this pane.
    ///
    /// Ported from the Mac's `lastSaid`, in the same words and for the same
    /// reason the notification body itself was: one person reads a banner on
    /// the Mac and a push on this phone about one pane, and they must not be
    /// two different notifications.
    ///
    /// `said` and NOT `recentSteps.last`. The two are cut from one message at
    /// opposite ends — a step is a wrapped row, so the last of them is the end
    /// of the window, while a notification arrives after the fact and has to
    /// open where the sentence opens. The cut is the host's; see
    /// `farcooler_core::feed::Feed::said`.
    ///
    /// The feed's last line is the fallback and only that: a runner still on
    /// an older daemon sends no `said`, and the tail of the window is a worse
    /// sentence than the head but a much better one than nothing.
    var lastSaid: String? {
        let quoted = (said ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        if !quoted.isEmpty { return quoted }
        return recentSteps.last
    }

    /// The subagents still running, named, at most three.
    ///
    /// Their COUNT is already inside `line`; these are the names, and three is
    /// what fits beside a row on a phone.
    var runningSubagents: [String] {
        Array((subagents ?? []).filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }.prefix(3))
    }

    /// How long the current state has been the state, for the two states where
    /// the answer changes what you do.
    ///
    /// An agent blocked for twenty minutes is a different situation from one
    /// blocked for ten seconds. "Idle for three days" is noise, so it is nil.
    func statusDuration(at now: Date) -> String? {
        guard agent == .blocked || agent == .working, let since = activitySince else { return nil }
        return Self.brief(secondsSince: since, at: now)
    }

    /// How long the whole turn has run. Does not restart when a permission
    /// prompt is approved, because saying yes to a tool call does not begin a
    /// new turn.
    func turnDuration(at now: Date) -> String? {
        guard let since = turnStartedAt else { return nil }
        return Self.brief(secondsSince: since, at: now)
    }

    private static func brief(secondsSince millis: Double, at now: Date) -> String? {
        let seconds = now.timeIntervalSince1970 - millis / 1000
        guard seconds >= 5 else { return nil }
        if seconds < 60 { return "\(Int(seconds))s" }
        if seconds < 3600 { return "\(Int(seconds / 60))m" }
        return "\(Int(seconds / 3600))h"
    }

    /// The one duration worth putting beside the status label.
    ///
    /// The two clocks answer different questions and conflating them is the bug
    /// they exist to fix. `Working` is only ever mid-turn, so the TURN clock is
    /// the honest answer to "how long has this been going". `Blocked` wants the
    /// STATE clock, because a prompt held for twenty minutes is the thing to
    /// notice, not how long the turn around it has run.
    ///
    /// `now` is an ARGUMENT rather than a `Date()` read inside, and that is
    /// what makes the string tick. Read inside, it is a value SwiftUI cannot
    /// observe: nothing about a working row changes from one second to the
    /// next, so the view is never invalidated and the duration freezes until
    /// something unrelated forces a redraw. Taken as an argument it is an input
    /// like any other, and a `TimelineView` supplying it once a second is a row
    /// that keeps its own time.
    func displayDuration(at now: Date) -> String? {
        agent == .working ? turnDuration(at: now) : statusDuration(at: now)
    }

    /// Where this terminal sorts. Absent `rank` sorts last: a daemon too old to
    /// send one is a daemon that cannot tell us this pane is urgent, and
    /// guessing that it is would put an unknown above a known blocked agent.
    var sortRank: UInt32 { rank ?? UInt32.max }

    /// What to call this terminal.
    ///
    /// Ported from the Mac app's `Terminal.label`, verbatim: derived, never
    /// stored, because a terminal IS the thing running in it. `preset`
    /// already carries what tmux reports is running — `claude`, `codex`,
    /// `zsh` — resolved on the host, because only the host has a screen to
    /// look at. The stored `title` used to be shown instead, and on a phone
    /// it read the same as on the Mac: "Terminal 12" with a counter that
    /// repeated after any removal, sitting next to the running command
    /// anyway — the only informative half of the row.
    var label: String {
        // The conversation's own name, when the agent has given it one.
        //
        // The Mac has read this since agent panes started reporting a title,
        // and the phone did not — so a fleet of agents all read "claude 1",
        // "claude 2", which is the one thing every pane has in common and
        // therefore says nothing about which is which.
        if !title.isEmpty, !Self.isPlaceholder(title) { return title }
        return Self.name(of: preset)
    }

    /// Whether a title is the automatic one every terminal is created with.
    private static func isPlaceholder(_ title: String) -> Bool {
        title.hasPrefix("Terminal ") || title == "Terminal"
    }

    /// `label`, plus its ordinal when it has one.
    ///
    /// The one place "claude" and "claude 2" are assembled into the single
    /// string a navigation title or a tab strip chip needs — `FleetView`
    /// keeps the two halves as separate `Text` views so it can dim the
    /// number, but a title bar and a tab chip have nowhere to hang a second
    /// view, so they get the joined form.
    func displayName(ordinal: Int?) -> String {
        // A named conversation needs no counter: the ordinal exists to tell
        // three identical "claude"s apart, and a title already has.
        guard let ordinal, label == Self.name(of: preset) else { return label }
        return "\(label) \(ordinal)"
    }

    /// One word for one thing, wherever a running command is shown.
    static func name(of command: String) -> String {
        let running = command.trimmingCharacters(in: .whitespaces).lowercased()
        if running.isEmpty { return "shell" }
        // The host reports whatever tmux sees running, so the same plain
        // shell arrives as `zsh` from a pane the watcher has looked at and as
        // `shell` from one it has not. Normalizing both to `shell` is what
        // keeps two identical shells from reading as different things.
        return shells.contains(running) ? "shell" : running
    }

    private static let shells: Set<String> = ["sh", "zsh", "bash", "fish", "dash", "ksh", "-zsh"]
}

/// What a coding agent is doing, as distinct from whether its process is alive.
///
/// `done` is idle that nobody has looked at yet — which is what makes it the
/// thing worth a notification, and what makes it clear itself when you open the
/// terminal.
enum AgentActivity: String {
    case none, idle, working, blocked, done, unknown

    static func parse(_ raw: String?) -> AgentActivity {
        guard let raw else { return .none }
        return AgentActivity(rawValue: raw) ?? .unknown
    }

    /// The single definition of "interrupt someone", shared with the Mac and
    /// with a future push notification or Live Activity.
    var wantsAttention: Bool { self == .blocked || self == .done }
    var isAgent: Bool { self != .none }

    var label: String {
        switch self {
        case .none: return ""
        case .idle: return "Idle"
        case .working: return "Working"
        case .blocked: return "Needs you"
        case .done: return "Done"
        case .unknown: return "Unknown"
        }
    }

    var symbol: String {
        switch self {
        case .none: return "terminal"
        case .idle: return "pause.circle"
        case .working: return "circle.dotted"
        case .blocked: return "hand.raised.fill"
        case .done: return "checkmark.circle.fill"
        case .unknown: return "questionmark.circle"
        }
    }
}

// `Fleet.landingTerminal` was here: an agent waiting on you, else the first
// terminal already running, else anything — the pane `FleetView` opened onto at
// connect. It is gone because the phone no longer lands on a terminal at all;
// it opens onto `NeedsYouView`, which lists everything wanting a person rather
// than picking one and hiding the rest. The ranking argument it embodied is not
// lost — the host computes it, on `Terminal.rank`, and the inbox orders by that.
//
// Android still has its own copy and still lands, which is fine: it mirrors the
// phone one release behind, and this is the release that changed.

struct Repository: Decodable, Identifiable, Hashable {
    var id: String
    var short: String
    var displayName: String
    var remote: String
}

struct RepositoryList: Decodable {
    var repositories: [Repository]
}

/// A directory the daemon is allowed to discover repositories under.
struct RepositoryRoot: Decodable, Identifiable, Hashable {
    var id: String
    /// Absent unless this client holds `host_admin`, which is the point: a
    /// read-scoped phone learns that a root exists without learning where on
    /// the runner it is. Shown as "Hidden" rather than as an empty row.
    var displayPath: String?
}

struct RepositoryRootList: Decodable {
    var roots: [RepositoryRoot]
}

/// A branch, for resuming onto work that already exists.
struct Branch: Decodable, Identifiable, Hashable {
    var name: String
    var local: Bool
    var remote: String?
    /// git refuses a second checkout of the same branch, so this has to be
    /// shown BEFORE somebody picks it — otherwise the only feedback is a
    /// failure after the fact.
    var checkedOut: Bool
    var subject: String
    /// When the branch's tip was last written, in Unix MILLISECONDS, or nil
    /// when git had no committer date for it.
    ///
    /// `Session::branches` has emitted this since the call existed and this
    /// side never declared it, so the phone's picker had no way to tell a
    /// branch from this morning from one abandoned in March — the two most
    /// useful facts about a branch you are choosing between are its name and
    /// its age, and only one of them was here. Not a decode failure; a field
    /// that arrived on every load and was dropped in silence, which is the
    /// quieter half of the same bug class as `isMainCheckout`.
    ///
    /// MILLISECONDS, and the unit is the trap. `Session::branches` sends
    /// `t.seconds * 1000` (`crates/client/src/session.rs:1085`); the CLI sends
    /// `t.seconds` under this same name (`crates/cli/src/main.rs:1646`), which
    /// is what the Mac's `BranchInfo.updatedAt` holds and why its `age`
    /// subtracts it from `timeIntervalSince1970` directly. Copying that
    /// arithmetic here would date every branch to 1970 and print an age in
    /// tens of thousands of days.
    ///
    /// Optional for the reason every field here is: a daemon too old to send
    /// it must cost the row its age, not the whole list its decode.
    var updatedAt: Double?

    var id: String { name }
    /// A branch that exists only on a remote still works: adopting one creates
    /// the local tracking branch. Worth labeling, because it is the difference
    /// between resuming your own work and picking up someone else's.
    var isRemoteOnly: Bool { !local && remote != nil }

    /// How long ago the tip was written, or empty when nobody said.
    ///
    /// The Mac's `BranchInfo.age` thresholds exactly — minutes under an hour,
    /// hours under a day, days after that — so one branch does not read as "3h"
    /// on a Mac and "today" on the phone beside it. The `/ 1000` is the whole
    /// difference between the two, and it is there because the producers differ
    /// rather than because the platforms do; see `updatedAt`.
    ///
    /// Empty rather than "unknown": a row that has nothing to say about age is
    /// better silent than captioned, and the caller draws nothing for "".
    func age(at now: Date) -> String {
        guard let updatedAt, updatedAt > 0 else { return "" }
        let seconds = now.timeIntervalSince1970 - updatedAt / 1000
        // A tip dated in the future is a clock disagreement between this phone
        // and the runner, not a branch from tomorrow. Say "now" rather than a
        // negative count of minutes.
        guard seconds > 0 else { return "now" }
        if seconds < 3600 { return "\(max(1, Int(seconds / 60)))m" }
        if seconds < 86_400 { return "\(Int(seconds / 3600))h" }
        return "\(Int(seconds / 86_400))d"
    }
}

struct BranchList: Decodable {
    var branches: [Branch]
}

/// One branch's place in a stack, and what GitHub says about it.
struct StackLink: Decodable, Identifiable, Hashable {
    var branch: String
    var parentBranch: String
    /// Only a guess is labeled. The other sources are recorded facts; a guessed
    /// parent produces a wrong diff that looks like a right one.
    var parentGuessed: Bool
    var ahead: Int
    var behind: Int
    var pr: PullRequest?

    var id: String { branch }
}

struct PullRequest: Decodable, Hashable {
    var number: Int
    var url: String
    var state: String
    var checks: String
    var review: String
    /// Read from GitHub long enough ago to doubt. Shown rather than hidden: a
    /// stale "passing" is the one reading that would mislead.
    ///
    /// Derived from `fetchedAt` by the DAEMON — fifteen minutes — so three
    /// platforms cannot disagree about what "a while ago" means. It arrived
    /// hardwired `false` until `99e78f8`, which is to say the sentence three
    /// apps have long drawn from it has only just become possible to see.
    var stale: Bool

    /// The PR head as GitHub last saw it.
    ///
    /// Optional, like every field added to these types after the first
    /// release — see `FleetDecodeTests`. A key an older runner never sends
    /// must leave a nil rather than fail the decode of the whole stack.
    var headOid: String?
    /// When it landed, in Unix MILLISECONDS, or nil while it has not.
    ///
    /// Milliseconds, and the unit is the trap `Branch.updatedAt` documents at
    /// length: this side of the wire sends epoch millis where the CLI sends
    /// seconds, and `Date(timeIntervalSince1970:)` on the raw number would
    /// date everything to 1970.
    var mergedAt: Double?
    /// When the daemon last read this from GitHub, in Unix MILLISECONDS.
    ///
    /// What `stale` is derived from, carried as well as the bool so a client
    /// can eventually say how long ago rather than only that it was a while.
    var fetchedAt: Double?
}

struct StackResponse: Decodable {
    var cycleDetected: Bool
    var links: [StackLink]

    /// Whether `gh` answered at all.
    ///
    /// "There is no pull request for this branch" and "we could not ask
    /// GitHub" both arrive as an absent `pr` on every link, and this is the
    /// only thing that separates them. It decides whether this app may offer
    /// to CREATE a pull request: offering that while a PR exists behind a
    /// logged-out `gh` is the app being confidently wrong about the one action
    /// on the row.
    ///
    /// It rides the LIST rather than each link because the fact being reported
    /// is "did `gh` answer", which is a property of the whole read.
    ///
    /// Optional, and every reader must go through `prAnswered`: a runner too
    /// old to send this is a runner that cannot tell us `gh` answered, which
    /// is exactly the case the offer must not be made in.
    var prKnown: Bool?
    /// The repository's page on GitHub, for building a compare link for a
    /// branch that has no pull request yet.
    ///
    /// Nil when `gh` has not answered for this repository. A client must get
    /// nothing rather than a link to nowhere.
    var repoUrl: String?

    /// Whether `gh` answered, read the only safe way. See `prKnown`.
    var prAnswered: Bool { prKnown == true }
}

// ---- what a pull request says on the branch header, where a test can read it
// ----
//
// Free of SwiftUI and out here rather than assembled inside a view body, for
// the reason Android's `Stack.kt` gives at the same place: there is no test
// bundle that can reach into a rendered row, so a sentence built inside one is
// a sentence nothing can check. The iOS app's only test bundle is a UI-testing
// one; AgentKit's runs on the host, which is why these live in this file.

/// How much the pull-request row should raise its voice.
///
/// Three levels and no more, because the row is a single line and the rule it
/// is built on is that a healthy pull request must be a grey line you skim
/// past. A signal that is always lit has stopped carrying information — the
/// position `71934f8` took when it turned the last permanent green dot
/// neutral, and the one Android's `PullRequestRows` took again for checks.
enum PullRequestEmphasis {
    /// Grey, and most of them. Open, approved, checks passed: nothing here
    /// wants you.
    case quiet
    /// Orange, and only ever this: nothing has been decided yet.
    case pending
    /// Red. The one thing on this header that should catch the eye.
    case alarm
}

extension PullRequest {
    /// Whether this pull request is still something anybody can act on.
    ///
    /// A merged or closed one is history: its checks and its review describe a
    /// decision already taken, so a red on either would be shouting about a
    /// question nobody is being asked.
    var isLive: Bool { state != "merged" && state != "closed" }

    /// The state half of the row's sentence, or nil when there is nothing to
    /// say.
    ///
    /// A draft or a landed PR names itself; an OPEN one is better described by
    /// where its review got to, which is the fact that decides what happens
    /// next. `review_required` cannot be shown raw — an underscore mid-sentence
    /// is a leaked wire value, which is what the Apple copy conventions rule
    /// out — and that is why this maps rather than capitalizes.
    var headerStateWord: String? {
        switch state {
        case "draft": return "Draft"
        case "merged": return "Merged"
        case "closed": return "Closed"
        case "open":
            switch review {
            case "approved": return "Approved"
            case "changes_requested": return "Changes requested"
            case "review_required": return "Review required"
            default: return "Open"
            }
        default: return nil
        }
    }

    /// The checks half, or nil where GitHub had nothing to report — a
    /// repository with no CI at all is most of them, and "Checks unknown" on
    /// every row would be a word spent to say nothing.
    var headerChecksWord: String? {
        switch checks {
        case "passing": return "Checks passed"
        case "failing": return "Checks failed"
        case "pending": return "Checks running"
        default: return nil
        }
    }

    /// The whole line, after the number.
    ///
    /// Two facts at most, in the order they are asked about: what state it is
    /// in, then whether it builds. Empty when GitHub told us neither, in which
    /// case the row is the number and the way through to it, which is still
    /// worth a line.
    var headerSentence: String {
        [headerStateWord, headerChecksWord].compactMap { $0 }.joined(separator: " · ")
    }

    /// See `PullRequestEmphasis`.
    var emphasis: PullRequestEmphasis {
        guard isLive else { return .quiet }
        if checks == "failing" || review == "changes_requested" { return .alarm }
        if checks == "pending" { return .pending }
        return .quiet
    }

    /// The one glyph the row may carry, or nil.
    ///
    /// Failing checks get it because they are the thing this row exists to
    /// make glanceable, and a merged PR gets one because it is notable without
    /// being actionable. Nothing else does: an icon on every row is a column
    /// of icons, and then none of them means anything.
    var headerSymbol: String? {
        if state == "merged" { return "arrow.triangle.merge" }
        if isLive && checks == "failing" { return "xmark.circle.fill" }
        return nil
    }
}

/// Ways into GitHub that this app builds rather than being handed.
enum PullRequestLink {
    /// GitHub's compare page, for a branch that has no pull request yet.
    ///
    /// Nil rather than a guess whenever any half is missing: `repoURL` is
    /// absent exactly when `gh` could not say what this repository is, and a
    /// button that opens a broken page is worse than no button.
    ///
    /// A branch compared against itself is nil too. The repository's own
    /// checkout sits on the base branch, and `main...main` is a compare page
    /// with nothing on it and a Create button that GitHub itself refuses.
    static func compare(repoURL: String?, baseRef: String, head: String) -> URL? {
        guard let repoURL, !repoURL.isEmpty, !head.isEmpty else { return nil }
        let base = branchName(baseRef)
        guard !base.isEmpty, base != head else { return nil }
        let root = repoURL.hasSuffix("/") ? String(repoURL.dropLast()) : repoURL
        let allowed = CharacterSet.urlPathAllowed
        guard
            let base = base.addingPercentEncoding(withAllowedCharacters: allowed),
            let head = head.addingPercentEncoding(withAllowedCharacters: allowed)
        else { return nil }
        // `expand=1` is what opens the compare page with the pull request form
        // already unfolded, which is the difference between landing on a diff
        // and landing on the thing you came to fill in.
        return URL(string: "\(root)/compare/\(base)...\(head)?expand=1")
    }

    /// A ref as GitHub names the branch behind it.
    ///
    /// `ChangeSet.baseRef` is a git ref and arrives as `origin/main` about as
    /// often as `main` — `resolve_base` hands back a recorded ref, a PR's base,
    /// or a default branch, and only some of those are remote-qualified. A
    /// compare URL takes branch NAMES, so the remote has to come off.
    ///
    /// Only a leading remote is dropped, not every component, and that is the
    /// difference from `default_branch`'s `rsplit('/')` in `review_ops.rs`:
    /// that one is peeling `origin/HEAD`, where the last component is the
    /// whole answer. Here `origin/release/2.0` is one branch on one remote,
    /// and rsplit would leave `2.0`, which names nothing.
    static func branchName(_ ref: String) -> String {
        var ref = ref
        for prefix in ["refs/heads/", "refs/remotes/"] where ref.hasPrefix(prefix) {
            ref.removeFirst(prefix.count)
        }
        // The two remote names a client can safely assume. Nothing on the wire
        // says what this repository's remotes are called, so a ref beginning
        // with anything else is left whole: a wrong strip invents a branch,
        // and a missed one is a compare page that still resolves.
        for remote in ["origin/", "upstream/"] where ref.hasPrefix(remote) {
            ref.removeFirst(remote.count)
        }
        return ref
    }
}

/// What the runner says about itself.
struct HostHealth: Decodable {
    var platform: String
    var daemonVersion: String
    var protocolVersion: Int
    var healthy: Bool
    /// The daemon's own words. Shown rather than summarized: this client cannot
    /// know which of them matters.
    var reasons: [String]
    var livePanes: Int
}

/// The states a terminal can be in, grouped by what a user should do about it.
enum StateKind {
    case starting, running, exited, error, lost, unknown

    static func parse(_ raw: String) -> StateKind {
        switch raw.lowercased() {
        case "starting": return .starting
        case "running": return .running
        case "exited": return .exited
        case "error": return .error
        case "lost": return .lost
        default: return .unknown
        }
    }

    /// Lost is red because it is the one state that means Far Cooler does not
    /// know what happened, and the user has to decide.
    var isAttentionWorthy: Bool { self == .lost || self == .error }
}

extension StateKind: Equatable {}
