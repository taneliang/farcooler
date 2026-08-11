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
    /// What the agent is doing, derived on the HOST. A phone has no screen to
    /// inspect, so this arriving over the wire is the only way it can know —
    /// and it is why the same badge means the same thing here as on the Mac.
    var activity: String?
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

extension Fleet {
    /// The terminal a host lands on when its worktree list is skipped — see
    /// `FleetView`. An agent waiting on you outranks everything else, because
    /// that is the whole reason to have opened the app; short of that, the
    /// first terminal already running is a better first screen than an
    /// arbitrary one that has exited or never started. `nil` only when the
    /// host has no terminals at all, which is what sends `FleetView` back to
    /// showing its list instead.
    var landingTerminal: Terminal? {
        let all = workspaces.flatMap(\.terminals)
        if let attention = all.first(where: { $0.agent.wantsAttention }) { return attention }
        if let running = all.first(where: { StateKind.parse($0.state) == .running }) { return running }
        return all.first
    }
}

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
    /// the machine it is. Shown as "Hidden" rather than as an empty row.
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

/// What the machine says about its own daemon.
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
