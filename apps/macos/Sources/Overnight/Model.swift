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
    var state: String
    var terminals: [Terminal]

    /// Terminals wanting the user, across this worktree.
    var attention: [Terminal] { terminals.filter(\.status.wantsAttention) }

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
            $0.title.lowercased().contains(q) || $0.preset.lowercased().contains(q)
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

    var agent: AgentActivity { AgentActivity.parse(activity) }

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

    /// One family, one size, one weight.
    ///
    /// The first attempt reached for literal imagery — a moon for idle, a
    /// gearwheel for working — which reads as a sticker sheet rather than a
    /// tool. These are all the same circle at the same optical weight, so a
    /// column of them lines up and the differences between them are the only
    /// thing that draws the eye.
    var symbol: String {
        switch self {
        case .starting: return "circle.dotted"
        case .running: return "circle.fill"
        case .idle: return "circle"
        case .working: return "circle.hexagonpath"
        case .blocked: return "exclamationmark.circle.fill"
        case .done: return "checkmark.circle.fill"
        case .exited: return "circle.slash"
        case .failed: return "xmark.circle.fill"
        case .lost: return "questionmark.circle.fill"
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
