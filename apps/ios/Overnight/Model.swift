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
