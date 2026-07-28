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
    var epoch: Int
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
