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

    enum CodingKeys: String, CodingKey {
        case runtimeHealthy = "runtime_healthy"
        case livePanes = "live_panes"
        case workspaces
    }

    static let empty = Fleet(runtimeHealthy: false, livePanes: 0, workspaces: [])
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
    /// Which machine it is on. Empty means this one.
    ///
    /// Deliberately NOT a filter. Hosts are a grouping, not a mode: you work
    /// across machines at once, and a picker that shows one at a time would
    /// make a remote agent something you have to go and look for rather than
    /// something already in front of you. It surfaces only where it
    /// disambiguates.
    var host: String?
    var worktree: String

    /// Whether this workspace IS the repository's own checkout.
    ///
    /// Adopted deliberately (`workspace main`) so a terminal can run there, and
    /// then never removable: the daemon refuses it, and the UI does not offer
    /// it. A refusal is a safety net, not a design — the button should not be
    /// there to press.
    ///
    /// Recognised by task name because that is what the daemon writes and what
    /// the fleet carries; the worktree path is not on every client.
    var isMainCheckout: Bool { task == "main" }
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
        // of one machine, saying which machine is noise.
        [repository, branch, (host?.isEmpty ?? true) ? nil : host]
            .compactMap { $0 }
            .filter { !$0.isEmpty }
            .joined(separator: " · ")
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
}

/// An allowlisted directory Overnight may operate under.
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

    /// Whether to offer the terminal/chat switch at all.
    var canSwitchPaneMode: Bool { chatCapable == true }

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
    /// A terminal used to carry two indicators — a coloured dot for whether the
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
            case .done: return .done
            }
        case .starting: return .starting
        case .exited: return .exited
        case .error: return .failed
        case .lost: return .lost
        default: return .running
        }
    }

    /// How long the current status has held, if that is worth knowing.
    ///
    /// Only for the two where duration changes what you do: an agent blocked
    /// for twenty minutes is a different situation from one blocked for ten
    /// seconds. "Idle for three days" is noise.
    var statusDuration: String? {
        guard status == .blocked || status == .working, let since = activitySince else {
            return nil
        }
        let seconds = Date().timeIntervalSince1970 - since / 1000
        guard seconds >= 5 else { return nil }
        if seconds < 60 { return "\(Int(seconds))s" }
        if seconds < 3600 { return "\(Int(seconds / 60))m" }
        return "\(Int(seconds / 3600))h"
    }
}

/// What a terminal's single indicator says.
///
/// Shape carries the meaning and colour reinforces it, never the other way
/// round: a colour-only indicator says nothing to a colourblind reader and
/// nothing at all in a screenshot.
enum Status: Equatable {
    case starting, running, idle, working, blocked, done, exited, failed, lost

    var label: String {
        switch self {
        case .starting: return "Starting"
        case .running: return "Running"
        case .idle: return "Idle"
        case .working: return "Working"
        case .blocked: return "Needs you"
        case .done: return "Done"
        case .exited: return "Exited"
        case .failed: return "Failed to start"
        case .lost: return "Lost"
        }
    }

    /// Does this want the user to do something?
    var wantsAttention: Bool {
        self == .blocked || self == .done || self == .lost || self == .failed
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
// The labels come from the daemon. The app maps them to colour and wording but
// never invents a state that the daemon did not report.

enum StateKind {
    case running, starting, exited, lost, error, ready, active, archived, unknown

    static func parse(_ s: String) -> StateKind {
        switch s {
        case "running": return .running
        case "starting": return .starting
        case "exited": return .exited
        case "LOST": return .lost
        case "ERROR", "error": return .error
        case "ready": return .ready
        case "active": return .active
        case "archived": return .archived
        default: return .unknown
        }
    }
}
