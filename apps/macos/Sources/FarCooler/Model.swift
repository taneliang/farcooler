import Foundation

// MARK: - Wire types
//
// These mirror the CLI's `--json` output. The Mac app is a THIN CLIENT: it
// renders state the daemon derived and never computes terminal state itself.
// A client that re-derives can disagree with the daemon and with other clients
// about the same terminal, which is the failure the design removed everywhere.

struct Fleet: Decodable, Equatable {
    var runtimeHealthy: Bool
    var livePanes: Int
    var workspaces: [Workspace]
    /// What this runner says a derived branch name starts with.
    ///
    /// Optional, by the rule stated on `Workspace` below: a client meeting an
    /// older daemon must not fail to decode the entire fleet over one absent
    /// key. `nil` and `""` are both "no prefix from this runner" here — the
    /// distinction between unset and deliberately empty is the daemon's to draw,
    /// and it has already drawn it by the time this arrives.
    var branchPrefix: String?

    enum CodingKeys: String, CodingKey {
        case runtimeHealthy = "runtime_healthy"
        case livePanes = "live_panes"
        case workspaces
        case branchPrefix = "branch_prefix"
    }

    static let empty =
        Fleet(runtimeHealthy: false, livePanes: 0, workspaces: [], branchPrefix: nil)
}

/// A worktree.
///
/// Fields added after the first release are OPTIONAL, not defaulted. Swift's
/// synthesized `Decodable` ignores default values and throws on a missing key,
/// so a client meeting an older daemon — or an app built before a CLI — would
/// fail to decode the entire fleet over one absent field, and show "no
/// workspaces" for a host full of them.
struct Workspace: Decodable, Identifiable, Hashable {
    var id: String
    var short: String
    var task: String
    var branch: String
    /// The project this worktree belongs to. Worktrees are grouped by it,
    /// because there are a handful of projects and potentially hundreds of
    /// worktrees across them.
    var repository: String?
    /// Which runner it is on. Empty means this one.
    ///
    /// Deliberately NOT a filter. Hosts are a grouping, not a mode: you work
    /// across runners at once, and a picker that shows one at a time would
    /// make a remote agent something you have to go and look for rather than
    /// something already in front of you. It surfaces only where it
    /// disambiguates.
    var host: String?
    var worktree: String

    /// Whether this workspace IS the repository's own checkout.
    ///
    /// From the daemon, which gets it from `git worktree list`. It used to be
    /// `task == "main"`, which a linked worktree in a directory called `main`
    /// would defeat — and what it guards is whether this app offers to delete
    /// the directory you work in.
    ///
    /// Optional because every field added after the first release is: a client
    /// meeting an older daemon must not fail to decode the entire fleet over
    /// one absent key and show "no workspaces" for a host full of them.
    var isMainCheckout: Bool { is_main_checkout ?? false }
    // swiftlint:disable:next identifier_name
    var is_main_checkout: Bool?

    /// The user asked not to see this one.
    var isHidden: Bool { state == "hidden" }

    /// git no longer lists this worktree, but the row carries terminals.
    var worktreeMissing: Bool { state == "worktree_missing" }
    var state: String
    var terminals: [Terminal]

    /// Terminals wanting the user, across this worktree.
    var attention: [Terminal] { terminals.filter(\.status.wantsAttention) }

    /// What the window says you are looking at.
    ///
    /// The task, then the project and branch beneath it. Not the focused
    /// terminal: with several panes on screen each one already names itself in
    /// its own header, so a title repeating one of them says less than nothing —
    /// it makes the window claim to be a terminal when it is a worktree. The
    /// place you are is the worktree, and that is true whether one pane is
    /// showing or four.
    var windowTitle: String { task }

    var windowSubtitle: String {
        // Host only where it disambiguates, same rule as the sidebar: on a fleet
        // of one runner, saying which runner is noise.
        [repository, branch, (host?.isEmpty ?? true) ? nil : host]
            .compactMap { $0 }
            .filter { !$0.isEmpty }
            .joined(separator: " / ")
    }

    /// A one-line summary for a collapsed row.
    ///
    /// A collapsed worktree still has to answer "is anything happening here?"
    /// or collapsing would hide the only thing the app is for.
    var summary: String {
        let agents = terminals.filter(\.agent.isAgent)
        if terminals.isEmpty { return "no terminals" }

        var parts: [String] = []
        if !agents.isEmpty {
            parts.append(agents.count == 1 ? "1 agent" : "\(agents.count) agents")
        }
        let others = terminals.count - agents.count
        if others > 0 {
            parts.append(others == 1 ? "1 shell" : "\(others) shells")
        }
        if !attention.isEmpty {
            parts.append(attention.count == 1
                ? attention[0].status.label.lowercased()
                : "\(attention.count) need you")
        } else if agents.contains(where: { $0.status == .working }) {
            parts.append("working")
        }
        return parts.joined(separator: " · ")
    }

    /// Does this worktree match a search?
    ///
    /// Matches its own name and branch AND its terminals', so typing an agent's
    /// name finds the worktree containing it. With worktrees unbounded, search
    /// is the primary way to reach one, not a convenience.
    func matches(_ query: String) -> Bool {
        let q = query.lowercased()
        if q.isEmpty { return true }
        if task.lowercased().contains(q) { return true }
        if branch.lowercased().contains(q) { return true }
        if (repository ?? "").lowercased().contains(q) { return true }
        if (host ?? "").lowercased().contains(q) { return true }
        return terminals.contains {
            $0.label.lowercased().contains(q)
        }
    }
}

struct Repository: Decodable, Identifiable, Hashable {
    var id: String
    var short: String
    var displayName: String
    var remote: String
    /// Which `RepositoryRoot` this repository is allowlisted under. Two
    /// repositories can share one root — the daemon only removes a whole
    /// root, never a single repository under it — so this is what lets the
    /// app warn about siblings before removing one takes the other with it.
    var repositoryRootId: String
}

/// An allowlisted directory Far Cooler may operate under.
///
/// `path` is optional in the protocol — it is returned only to a host_admin
/// client — so the model has to allow its absence rather than assume it.
struct RepositoryRoot: Decodable, Identifiable, Hashable {
    var id: String
    var short: String
    var path: String?
    var repositories: Int
}

struct RootList: Decodable {
    var roots: [RepositoryRoot]
}

struct RepositoryList: Decodable {
    var repositories: [Repository]
}

struct Terminal: Decodable, Identifiable, Hashable {
    var id: String
    var short: String
    var title: String
    var preset: String
    var state: String
    /// What the AGENT is doing, as the daemon derived it. Absent on older
    /// daemons, which is why it is optional rather than defaulted to something
    /// that would look like a real answer.
    var activity: String?
    /// Unix milliseconds when the activity last changed, from the daemon.
    ///
    /// Timed on the host rather than by the client, so "working for 4m" does
    /// not restart at every reconnect or lie after a laptop sleeps.
    var activitySince: Double?
    /// How the command ended. Absent while it is still running, and absent from
    /// older daemons — which is why a missing value is never read as a failure.
    var exitCode: Int?
    var exitSignal: Int?
    /// Unix milliseconds when the current TURN started, from the daemon.
    var turnStartedAt: Double?
    /// What the agent is asking, when it is blocked.
    var blockedQuestion: String?
    /// The last three DISPLAY LINES of what the agent has been saying, oldest
    /// first.
    ///
    /// A transcript, and only a transcript: the agent's own prose, verbatim,
    /// with no verb in front of it. What it DID is not here — a tool call is
    /// where the agent IS rather than what it told you, and it arrives on
    /// `line` instead. This field used to carry both, prefixed with the tool's
    /// own lowercased name, which is how a row came to read `taskupdate`.
    ///
    /// Lines rather than messages, and that is the whole of it: a long
    /// sentence spans two or three of these, and new prose pushes old prose
    /// off the top the way watching a terminal does. It used to be the last
    /// three MESSAGES with each cut to forty characters, which read as three
    /// fragments and none of them a sentence.
    ///
    /// Finished lines, not structured steps: the daemon reads the agent's own
    /// session log, redacts each one and wraps it to the width of a row before
    /// it ever reaches here, so a Mac, a phone and a watch cannot each break a
    /// line somewhere different. This app renders them and decides nothing
    /// about them.
    ///
    /// Optional because a daemon from before this existed sends no key at all,
    /// and a row with no feed has to read as "nothing to say" rather than as a
    /// decoding failure.
    var feed: [String]?
    /// The last thing the agent said, WHOLE and from its opening.
    ///
    /// The same message `feed`'s last lines were cut from, cut from the other
    /// end and to a notification's width rather than a row's. It is a separate
    /// field because it cannot be recovered from those lines: a feed entry is
    /// a wrapped ROW, so the last of them is the last forty characters of the
    /// window — which is how a banner about a turn that ended "More shit. An
    /// industrial quantity of shit, shipped in carefully authorized batches to
    /// avoid N+1 shits." came to read "batches to avoid N+1 shits."
    ///
    /// Cut on the host, like every other string here, and to about 120
    /// characters — see `farcooler_core::feed::SAID_WIDTH` for why that
    /// number. This app renders it and decides nothing about it.
    ///
    /// Optional because a daemon from before this existed sends no key, which
    /// `lastSaid` reads as "ask the feed instead" rather than as nothing said.
    var said: String?
    /// Where the agent is, in one line: the question it is blocked on, its
    /// position in its own task list (`3/7 · Designing test matrix · 2
    /// agents`), or what it is doing right now (`Writing fruit.txt`).
    ///
    /// One rung of the daemon's compact ladder, and the one a sidebar row has
    /// room for. The priority between those three is decided on the host — see
    /// `farcooler_core::feed::line` — because a phone, a watch and this app
    /// deciding it separately is three surfaces disagreeing about one pane.
    ///
    /// Optional for the same reason `feed` is.
    var line: String?
    /// The agents this agent spawned and has not finished with, named by the
    /// short description it spawned them with.
    ///
    /// Running only, and already redacted and cut to a row's width like every
    /// other line here. Their COUNT is already inside `line`; these are the
    /// names, for a surface with room to list them.
    var subagents: [String]?
    /// Whether the turn the agent just finished DIED rather than completed.
    ///
    /// Read from the agent's own session log on the host — a cursor turn that
    /// came back "Named models unavailable" ends with an error status — and
    /// carried beside `activity`, which has no word for it: a turn that died
    /// and one that succeeded are both `done` there. Without this the row said
    /// "Done" with a green dot for an agent that had stopped working, which is
    /// the most expensive thing a fleet at a glance can get wrong.
    ///
    /// Optional because a daemon from before this existed sends no key, and
    /// absent means "nothing claimed the turn went badly" rather than "it did".
    var turnFailed: Bool?
    var epoch: Int
    /// What this terminal's pane is hosting. Absent on older daemons, which is
    /// why it is optional rather than defaulted to something that would look
    /// like a real answer.
    var paneMode: String?
    var agentSessionId: String?
    var agentMode: String?
    var availableAgentModes: [String]?
    /// Whether this pane can be rendered as a chat.
    ///
    /// Answered on the host: identifying an agent takes a screen read, because
    /// Claude Code renames its own process. Absent from older daemons, and
    /// absent means "do not offer" rather than "yes" — offering a switch that
    /// comes back as a different agent is worse than not offering one.
    var chatCapable: Bool?

    var agent: AgentActivity { AgentActivity.parse(activity) }

    /// Whether to draw a chat or a VT grid.
    var isAgentPane: Bool { paneMode == "agent" }

    /// Whether this pane is this worktree's diff.
    ///
    /// A real tmux pane like any other — it splits, drags, zooms and breaks out
    /// — whose contents this app draws instead of a terminal grid. The process
    /// behind it exists only to hold the rectangle; see `farcooler pane-host`.
    var isChangesPane: Bool { paneMode == "changes" }

    /// Whether to offer the terminal/chat switch at all.
    ///
    /// Never on a changes pane. There is no TUI underneath it to switch back
    /// to, and the daemon refuses the call — offering a control that can only
    /// produce an error message is worse than not offering it.
    var canSwitchPaneMode: Bool { chatCapable == true && !isChangesPane }

    /// Whether an agent is running here at all, as opposed to a plain shell.
    ///
    /// The same distinction the daemon's own refusal draws — `"nothing in
    /// this pane is an agent"` versus `"this agent has no chat adapter"` —
    /// so the client can tell the two apart too: a shell that never ran an
    /// agent and an agent that ran but has no adapter fail for different
    /// reasons, and only one of them is fixed by writing a `config.toml`
    /// entry.
    ///
    /// A *live* plain shell pane reports its real process name — `zsh`,
    /// `fish`, `bash` — not the literal string `"shell"`; the daemon only
    /// collapses to `"shell"` for a pane it read as empty or all-numeric.
    /// Testing against the literal string alone would tell a zsh pane it
    /// has no chat adapter and to go add one to `config.toml`, which is
    /// exactly the bad advice this distinction exists to avoid. So this
    /// checks the same `shells` set `name(of:)` uses to fold process names
    /// to one word, instead of re-deriving that knowledge here.
    var hasDetectedAgent: Bool {
        let name = preset.split(separator: ":").first.map(String.init) ?? ""
        guard !name.isEmpty else { return false }
        return name != "shell" && !Self.shells.contains(name.lowercased())
    }

    /// What is running here, for a sentence about it.
    ///
    /// The preset when there is one — "codex", "cursor" — and something
    /// truthful rather than a guess when there is not.
    var agentLabel: String {
        guard hasDetectedAgent else { return "This pane" }
        let name = preset.split(separator: ":").first.map(String.init) ?? ""
        return name.prefix(1).uppercased() + name.dropFirst()
    }

    /// What to call this terminal.
    ///
    /// Derived, never stored, and that is the whole point: a terminal IS the thing
    /// running in it. `preset` already carries what tmux reports is running —
    /// `claude`, `codex`, `zsh` — resolved on the host, because only the host has a
    /// screen to look at.
    ///
    /// The stored title used to be shown instead, and it was noise with a counter
    /// attached. "Terminal 12" says nothing you cannot see, the counter came from
    /// the terminal COUNT so it repeated after any removal — three different panes
    /// called "Terminal 12" — and beside it sat the running command anyway, which
    /// was the only informative half of the row.
    /// The agent's own name for the conversation, when it has one.
    ///
    /// A fleet of eight panes all reading "claude" says nothing about which is
    /// which — the process is identical in every one of them, and what differs
    /// is the WORK. The agent names its conversation after that and revises it
    /// as the work changes, which is the only description of a pane that is
    /// about the task rather than the program.
    ///
    /// Only used when the daemon has one to give. The stored title is rejected
    /// here for the reason below: it is "Terminal 12", which is a counter.
    var label: String {
        // Named for what it is, not for what is running in it. A changes pane's
        // process is `farcooler`, and the daemon stores whatever title the
        // split was asked for — neither is the word a person reading the header
        // needs, and both are answers to a question about a program rather than
        // about the pane.
        if isChangesPane { return "Changes" }
        if !title.isEmpty, !Self.isPlaceholder(title) { return title }
        return Self.name(of: preset)
    }

    /// Whether a title is the automatic one every terminal is created with.
    private static func isPlaceholder(_ title: String) -> Bool {
        title.hasPrefix("Terminal ") || title == "Terminal"
    }

    /// One word for one thing, wherever a running command is shown.
    ///
    /// Shared with the layout tabs, which name a layout after what is in it and
    /// were otherwise reading `zsh` off the same panes the sidebar called `shell`.
    static func name(of command: String) -> String {
        let running = command.trimmingCharacters(in: .whitespaces).lowercased()
        if running.isEmpty { return "shell" }
        // One word for one thing. The host reports whatever tmux sees running, so
        // the same plain shell arrives as `zsh` from a pane the watcher has looked
        // at and as `shell` from one it has not — and a sidebar listing `zsh 1`,
        // `shell 2`, `zsh 3` for three identical shells is worse than no name,
        // because it implies a difference that is not there.
        //
        // Agents keep their own names: `claude` and `codex` are the distinction
        // that matters, and the whole point of deriving the label is to surface it.
        return shells.contains(running) ? "shell" : running
    }

    private static let shells: Set<String> = ["sh", "zsh", "bash", "fish", "dash", "ksh", "-zsh"]

    /// The ONE thing this terminal's indicator should say.
    ///
    /// A terminal used to carry two indicators — a colored dot for whether the
    /// process was alive, and a glyph for what the agent was doing. They
    /// competed for the same glance and forced a reader to learn two
    /// vocabularies for one row. They are not independent: activity only means
    /// anything while the process is running, so one derives from the other.
    var status: Status {
        switch StateKind.parse(state) {
        case .running:
            // Running is implied and uninteresting. What the agent is doing is
            // the answer to the question you actually asked.
            switch agent {
            case .none, .unknown: return .running
            case .idle: return .idle
            case .working: return .working
            case .blocked: return .blocked
            // Finished, and how it finished. `done` is the daemon's word for
            // "the turn ended and nobody has looked yet", which is true of a
            // turn that died as well as one that worked — the log says which,
            // and a row that drew both the same way told the reader everything
            // was fine when it was not.
            case .done: return turnFailed == true ? .failedTurn : .done
            }
        case .starting: return .starting
        case .exited:
            // A non-zero code or a signal is a failure worth seeing; a clean
            // exit is not. An ABSENT code is not a failure either — an older
            // daemon sends none, and reading that as broken would mark every
            // finished terminal on the runner.
            if exitSignal != nil || (exitCode.map { $0 != 0 } ?? false) {
                return .failedRun
            }
            return .exited
        case .error: return .failed
        case .lost: return .lost
        // `StateKind.unknown` is both the daemon's `unknown` and anything this
        // build does not recognize, and neither may fall through to `.running`
        // — which is what `default` used to do. A state we cannot read being
        // drawn as a healthy process is the one mistake the derivation rule is
        // built to make impossible, and the app was quietly reintroducing it.
        case .unknown: return .unreadable
        default: return .running
        }
    }

    /// How long the current status has held, if that is worth knowing.
    ///
    /// Only for the two where duration changes what you do: an agent blocked
    /// for twenty minutes is a different situation from one blocked for ten
    /// seconds. "Idle for three days" is noise.
    func statusDuration(at now: Date) -> String? {
        guard status == .blocked || status == .working, let since = activitySince else {
            return nil
        }
        return Self.brief(secondsSince: since, at: now)
    }

    /// How long the whole turn has been running.
    ///
    /// Distinct from `statusDuration`, and the one a person means by "how long
    /// has this been going". It does not restart when a permission prompt is
    /// approved, because saying yes to a tool call does not begin a new turn.
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
    /// "Working 12m" answers a different question from "Needs you 2m", and
    /// conflating them is the bug the two clocks exist to fix: `Working` is
    /// only ever mid-turn, so the turn clock is the honest answer to "how
    /// long has this been going"; `Blocked` wants the state clock, because a
    /// permission prompt held for twenty minutes is the thing to notice, not
    /// how long the turn around it has run.
    ///
    /// `now` is an ARGUMENT rather than a `Date()` read inside, and that is
    /// what makes the string tick. Read inside, it is a value SwiftUI has no
    /// way to observe: nothing about a working row changes from one second to
    /// the next, so the view is never invalidated and the duration freezes
    /// until some unrelated event forces a redraw — clicking another pane,
    /// which is exactly how the complaint was phrased. Taken as an argument it
    /// is an input like any other, and a `TimelineView` supplying it once a
    /// second is a row that keeps its own time. See `Ticking`.
    func displayDuration(at now: Date) -> String? {
        status == .working ? turnDuration(at: now) : statusDuration(at: now)
    }

    /// The one line that says where this agent is.
    ///
    /// The question it is blocked on, its position in its own task list, or
    /// what it is doing right now — decided on the host, in that order, so a
    /// phone and a watch and this sidebar cannot each pick a different one. See
    /// `farcooler_core::feed::line`.
    ///
    /// Agents only. The daemon fills the same rung for a plain shell with
    /// `zsh · running · 5s`, which is the row's own name and status read back
    /// to it — useful to a watch face that has nothing else, and noise under
    /// every shell in a sidebar that already draws both.
    var signalLine: String? {
        guard agent.isAgent else { return nil }
        let text = line?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return text.isEmpty ? nil : text
    }

    /// The agents this one has running, named.
    ///
    /// Trimmed and emptied out for the same reason `recentSteps` is: a blank
    /// line makes the row taller and says nothing. Capped for the same reason
    /// too — a fleet of twenty background agents must not turn one sidebar row
    /// into a screenful.
    var runningSubagents: [String] {
        let named = (subagents ?? [])
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        return Array(named.prefix(3))
    }

    /// The steps worth drawing under this row.
    ///
    /// NOT gated on the status, unlike `signalLine` above, and that is the
    /// difference between the two: where an agent IS stops being true the
    /// moment it moves, but what it said stays true afterwards. "What did it
    /// do while I was away" is exactly the moment the summary is worth most,
    /// so an idle or finished agent keeps its lines — and a row that shed them
    /// on going idle would also mean the sidebar rearranging itself under
    /// someone reading it.
    ///
    /// Trimmed and emptied-out here rather than trusted whole: an empty string
    /// would still draw a blank line and make the row taller for nothing.
    var recentSteps: [String] {
        let steps = (feed ?? [])
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        // The daemon already keeps only three; the cap is repeated here so a
        // host that ever sent four could not make one row twice the height of
        // every other row in the fleet.
        return Array(steps.suffix(3))
    }

    /// What to quote in a notification about this pane.
    ///
    /// `said` and NOT `recentSteps.last`, which is what a banner used to
    /// quote. The two are cut from one message at opposite ends: a step is a
    /// wrapped row, so the last of them is the end of the window, and a
    /// notification arrives after the fact and has to open where the sentence
    /// opens. That is decided on the host — see
    /// `farcooler_core::feed::Feed::said` — so this Mac, the phone, and the
    /// push the daemon sends when neither app is watching all quote one
    /// string.
    ///
    /// The feed's last line is the fallback and only that: a runner still on
    /// an older daemon sends no `said`, and the tail of the window is a worse
    /// sentence than the head but a much better one than nothing.
    var lastSaid: String? {
        let quoted = (said ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        if !quoted.isEmpty { return quoted }
        return recentSteps.last
    }
}

/// What a terminal's single indicator says.
///
/// Shape carries the meaning and color reinforces it, never the other way
/// round: a color-only indicator says nothing to a colorblind reader and
/// nothing at all in a screenshot.
enum Status: Equatable {
    case starting, running, idle, working, blocked, done, exited, failed, lost
    /// The command ended badly.
    ///
    /// Distinct from `exited`, which is a shell you closed or a server you
    /// stopped and is not news. A build that failed overnight is the reason to
    /// look at the fleet at all.
    case failedRun
    /// The agent's turn ended badly.
    ///
    /// The same news as `failedRun`, one layer up: the process is alive and
    /// perfectly healthy, and the WORK died. Kept as its own case rather than
    /// folded into `failedRun` because the two are announced differently — a
    /// command has an exit code to quote and an agent has a turn — and because
    /// a terminal can only ever be one of them, so sharing a case would only
    /// have saved a line at the cost of the notification quoting a code that
    /// does not exist.
    case failedTurn
    /// The runner did not answer, so this row is not saying anything.
    ///
    /// Distinct from `lost`, which is a finding the daemon actually made. This
    /// is the daemon reporting that it could not look: tmux answers one request
    /// at a time per server, and a single pane whose program has stopped
    /// reading its input can stall every read queued behind it.
    ///
    /// Shown rather than papered over with the last good value, because a row
    /// that goes on claiming "Running" while nothing can be read is exactly the
    /// failure this vocabulary exists to prevent. It is simply a different
    /// claim from "this terminal died".
    case unreadable

    var label: String {
        switch self {
        case .starting: return "Starting"
        case .running: return "Running"
        case .idle: return "Idle"
        case .working: return "Working"
        case .blocked: return "Needs you"
        case .done: return "Done"
        case .exited: return "Exited"
        case .failedRun, .failedTurn: return "Failed"
        case .failed: return "Failed to start"
        case .lost: return "Lost"
        case .unreadable: return "Not answering"
        }
    }

    /// Does this want the user to do something?
    ///
    /// `unreadable` deliberately does not. There is nothing to do about it, and
    /// it resolves on its own far more often than not — counting it would put a
    /// badge on the window every time a pane took a moment to answer.
    var wantsAttention: Bool {
        self == .blocked || self == .done || self == .lost || self == .failed
            || self == .failedRun || self == .failedTurn
    }

    /// Should it move?
    ///
    /// Motion is the strongest signal a UI has, so it belongs only on the thing
    /// that is genuinely still changing. A static badge that pulses is noise.
    var animates: Bool { self == .working || self == .starting }
}

/// What a coding agent is doing, as distinct from whether its process is alive.
///
/// `done` is not a state of the agent — it is idle that nobody has looked at
/// yet. That is what makes it worth a notification and what makes it clear
/// itself when you open the terminal.
enum AgentActivity: String {
    case none, idle, working, blocked, done, unknown

    static func parse(_ raw: String?) -> AgentActivity {
        guard let raw else { return .none }
        return AgentActivity(rawValue: raw) ?? .unknown
    }

    /// Is this worth interrupting someone for?
    ///
    /// One definition, so a badge, a notification and a future Live Activity
    /// cannot disagree about what deserves attention.
    var wantsAttention: Bool { self == .blocked || self == .done }

    /// Nothing to show for a plain shell — putting it in the same visual
    /// language as an agent is noise in the list you scan for what needs you.
    var isAgent: Bool { self != .none }

}

// MARK: - State vocabulary
//
// The labels come from the daemon. The app maps them to color and wording but
// never invents a state that the daemon did not report.

enum StateKind {
    case running, starting, exited, lost, error, ready, active, hidden, worktreeMissing, unknown

    static func parse(_ s: String) -> StateKind {
        switch s {
        case "running": return .running
        case "starting": return .starting
        case "exited": return .exited
        case "LOST": return .lost
        case "ERROR", "error": return .error
        case "ready": return .ready
        case "active": return .active
        case "hidden": return .hidden
        case "worktree_missing": return .worktreeMissing
        default: return .unknown
        }
    }
}
