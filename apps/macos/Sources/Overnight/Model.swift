import Foundation

// MARK: - Wire types
//
// These mirror the CLI's `--json` output. The Mac app is a THIN CLIENT: it
// renders state the daemon derived and never computes terminal state itself.
// A client that re-derives can disagree with the daemon and with other clients
// about the same terminal, which is the failure the design removed everywhere.

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
    var worktree: String
    var state: String
    var terminals: [Terminal]
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
    var epoch: Int

    var agent: AgentActivity { AgentActivity.parse(activity) }
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
