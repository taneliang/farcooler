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
    var epoch: Int
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
