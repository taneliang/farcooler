import Foundation

// The overview's workspace level: within each runner's section, a heading per
// workspace — its Board row, its orchestrator, then its worktrees — and an
// Unclaimed heading at the end of each repository for the worktrees no
// workspace owns.
//
// The phone grouped by runner and nothing else, so this is new layout under an
// old heading rather than a regrouping: the runner stays the outer level, and
// a runner without `workstreams` keeps exactly the section it had (`layout`
// answers nil for it). The grouping itself is `WorkspaceGrouping`'s, shared
// with the Mac and mirrored by Android's `model/FleetLayout.kt`; what this
// file adds is what the phone DRAWS from it, which is a rule a view body would
// otherwise decide and no suite would read. See `ShellWorkspacesTests`.

/// One heading inside a runner's section.
struct ShellWorkspaceHeading: Identifiable, Hashable, Sendable {
    /// The workspace's id, or `unclaimed/<repository id>` for a repository's
    /// Unclaimed group — never a workspace's id, so a Board row can never be
    /// drawn under it.
    var id: String
    /// What the heading says: the workspace's name, or "Unclaimed".
    var name: String
    /// The repository's display name, when the runner has more than one
    /// repository and the heading would otherwise not say which it is under.
    /// Nil for the common case of one, where it would be the same word over
    /// every heading.
    var repository: String?
    /// Whether this is a repository's Unclaimed group.
    var isUnclaimed: Bool = false
    /// The workspace's orchestrator, drawn as the heading's own row and in no
    /// worktree's card. Nil when none is running, and always for Unclaimed.
    var orchestrator: ShellOrchestratorRow?

    static func unclaimedID(repository: String) -> String { "unclaimed/\(repository)" }
}

/// A workspace's orchestrator, as its heading draws it.
struct ShellOrchestratorRow: Hashable, Sendable {
    /// What opening it hands back: the terminal's id in the app, a tab's id in
    /// the harness.
    var id: String
    /// What the row says after the name, the pane's own one-line summary. Nil
    /// draws the name alone.
    var line: String?
    var mark: GlanceMark
}

/// A runner's fleet laid out by workspace: the headings in drawing order, and
/// which heading each worktree is under.
struct ShellWorkspaceLayout: Equatable, Sendable {
    struct Heading: Equatable, Sendable {
        var heading: ShellWorkspaceHeading
        /// The orchestrator's terminal id; the caller turns it into a row.
        var orchestrator: String?
        /// The daemon's worktree ids, in the runner's order.
        var worktrees: [String]
    }

    var headings: [Heading]

    /// The daemon's worktree ids, in the order the overview draws them. The
    /// shell's fleet is built in this order, so a swipe through the bar walks
    /// the grid in the order it is drawn.
    var order: [String] { headings.flatMap(\.worktrees) }

    /// The heading each worktree is under, by the daemon's worktree id.
    var headingOf: [String: String] {
        var out: [String: String] = [:]
        for heading in headings {
            for worktree in heading.worktrees { out[worktree] = heading.heading.id }
        }
        return out
    }

    /// Every orchestrator's terminal id: the panes no worktree's card shows.
    var orchestrators: Set<String> { Set(headings.compactMap(\.orchestrator)) }

    /// What each orchestrator's tab is called, by its terminal id: "Billing
    /// Orchestrator". Its pane is a tab of the main checkout, which another
    /// workspace may own, and a tab called after its program would read as
    /// one more terminal of that worktree's in the bar.
    var orchestratorTitles: [String: String] {
        var out: [String: String] = [:]
        for heading in headings {
            if let id = heading.orchestrator {
                out[id] = ShellTab.orchestratorTitle(workspace: heading.heading.name)
            }
        }
        return out
    }
}

extension ShellTab {
    /// An orchestrator's tab title: its workspace's name, then "Orchestrator",
    /// as its row under that workspace's heading says.
    static func orchestratorTitle(workspace: String) -> String { "\(workspace) Orchestrator" }
}

extension Fleet {
    /// This runner's fleet laid out by workspace, or nil for a runner without
    /// `workstreams`, which keeps the flat section it always had.
    ///
    /// - Repositories in the order `repositoryGroups` gives, and within each,
    ///   Main first and then by ordinal. Every workspace gets a heading, one
    ///   with no worktrees too: it still has a board and may have an
    ///   orchestrator, and the model should be visible before the first split.
    /// - A repository's Unclaimed heading comes after its workspaces, and only
    ///   when something is unclaimed.
    /// - An orchestrator is the heading's, and only when its terminal is in
    ///   this fleet: a `WorkspaceSummary.orchestrator` naming a pane the fleet
    ///   has not listed would be a row that opens nothing.
    /// - `names` are repository display names by id. A heading names its
    ///   repository only when the runner has more than one.
    func shellLayout(names: [String: String]) -> ShellWorkspaceLayout? {
        guard workspaces != nil else { return nil }
        let groups = repositoryGroups()
        let terminals = Set(worktrees.flatMap { $0.terminals.map(\.id) })
        let several = groups.count > 1
        var headings: [ShellWorkspaceLayout.Heading] = []
        for repository in groups {
            let label = several ? names[repository.repository] : nil
            for group in repository.workspaces {
                headings.append(
                    .init(
                        heading: ShellWorkspaceHeading(
                            id: group.workspace.id, name: group.workspace.name,
                            repository: label),
                        orchestrator: group.orchestrator.flatMap { terminals.contains($0) ? $0 : nil },
                        worktrees: group.worktrees))
            }
            if !repository.unclaimed.isEmpty {
                headings.append(
                    .init(
                        heading: ShellWorkspaceHeading(
                            id: ShellWorkspaceHeading.unclaimedID(repository: repository.repository),
                            name: "Unclaimed", repository: label, isUnclaimed: true),
                        orchestrator: nil,
                        worktrees: repository.unclaimed))
            }
        }
        return ShellWorkspaceLayout(headings: headings)
    }
}
