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
    var task: String
    var branch: String
    var worktree: String?
    var state: String
    var terminals: [Terminal]

    /// Which of several identically-labelled terminals each one is, keyed by
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

    var agent: AgentActivity { AgentActivity.parse(activity) }

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
    var label: String { Self.name(of: preset) }

    /// `label`, plus its ordinal when it has one.
    ///
    /// The one place "claude" and "claude 2" are assembled into the single
    /// string a navigation title or a tab strip chip needs — `FleetView`
    /// keeps the two halves as separate `Text` views so it can dim the
    /// number, but a title bar and a tab chip have nowhere to hang a second
    /// view, so they get the joined form.
    func displayName(ordinal: Int?) -> String {
        guard let ordinal else { return label }
        return "\(label) \(ordinal)"
    }

    /// One word for one thing, wherever a running command is shown.
    static func name(of command: String) -> String {
        let running = command.trimmingCharacters(in: .whitespaces).lowercased()
        if running.isEmpty { return "shell" }
        // The host reports whatever tmux sees running, so the same plain
        // shell arrives as `zsh` from a pane the watcher has looked at and as
        // `shell` from one it has not. Normalising both to `shell` is what
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

    /// Lost is red because it is the one state that means Overnight does not
    /// know what happened, and the user has to decide.
    var isAttentionWorthy: Bool { self == .lost || self == .error }
}

extension StateKind: Equatable {}
