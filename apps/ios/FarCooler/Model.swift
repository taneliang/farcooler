import Foundation

// The shapes the client core returns. Identical to the Mac app's, because both
// decode what one Rust crate produces — there is one definition of what a
// workspace looks like on the wire, not one per platform.

struct Fleet: Decodable {
    var runtimeHealthy: Bool
    var livePanes: Int
    var workspaces: [Workspace]

    enum CodingKeys: String, CodingKey {
        case runtimeHealthy = "runtime_healthy"
        case livePanes = "live_panes"
        case workspaces
    }

    static let empty = Fleet(runtimeHealthy: false, livePanes: 0, workspaces: [])
}

struct Workspace: Decodable, Identifiable, Hashable {
    var id: String
    var short: String
    /// Which repository this worktree belongs to.
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
    /// lives in. Optional because an older daemon never sent this key, and
    /// decoding must not fail the entire fleet over one absent field.
    var isMainCheckout: Bool { is_main_checkout ?? false }
    // swiftlint:disable:next identifier_name
    var is_main_checkout: Bool?

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

    var id: String { name }
    /// A branch that exists only on a remote still works: adopting one creates
    /// the local tracking branch. Worth labeling, because it is the difference
    /// between resuming your own work and picking up someone else's.
    var isRemoteOnly: Bool { !local && remote != nil }
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
    var stale: Bool
}

struct StackResponse: Decodable {
    var cycleDetected: Bool
    var links: [StackLink]
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
