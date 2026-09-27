import Foundation

// Workspaces as the apps group by them: repository, then workspace, then its
// worktrees, with the worktrees no workspace owns in an Unclaimed group last.
//
// One rule, public, so the Mac (which decodes the CLI) and the iPhone (which
// decodes the client core) group the same fleet the same way. Android's
// `model/WorkspaceGroups.kt` mirrors it line for line.
//
// A runner without the `workstreams` capability has no workspaces. It gets
// one implicit workspace per repository holding every worktree, which is the
// layout from before workspaces existed. `WorkspaceSummary.isImplicit` is
// what lets a view leave the workspace level out for it.

/// One workspace: a workstream with a name, a task prefix and a board.
///
/// Decoded from the objects under the fleet's `workspaces` and from
/// `farcooler workspace list --json`, which `workspaces_json` in
/// `crates/client` writes for both. Snake_case keys, as that module's are.
public struct WorkspaceSummary: Identifiable, Equatable, Hashable, Sendable, Decodable {
    public let id: String
    /// The repository this workspace belongs to, as the producer sends it:
    /// a UUID string from both `Session::fleet` and the CLI's workspace list.
    /// Nil only from a producer that left it out.
    public let repository: String?
    public let name: String
    /// What new keys on this board start with: `bil` in `bil-3`.
    public let taskPrefix: String
    public let isMain: Bool
    /// Order within the repository, as the runner stores it. Main is 0.
    public let ordinal: Int
    /// The live orchestrator's terminal id, or nil when none is running.
    public let orchestrator: String?
    /// Whether this stands in for a runner without workspaces: one per
    /// repository, whose id is the repository's own. Never decoded.
    public let isImplicit: Bool

    public init(
        id: String, name: String, taskPrefix: String, isMain: Bool, ordinal: Int,
        repository: String? = nil, orchestrator: String? = nil
    ) {
        self.init(
            id: id, repository: repository, name: name, taskPrefix: taskPrefix,
            isMain: isMain, ordinal: ordinal, orchestrator: orchestrator, isImplicit: false)
    }

    private init(
        id: String, repository: String?, name: String, taskPrefix: String, isMain: Bool,
        ordinal: Int, orchestrator: String?, isImplicit: Bool
    ) {
        self.id = id
        self.repository = repository
        self.name = name
        self.taskPrefix = taskPrefix
        self.isMain = isMain
        self.ordinal = ordinal
        self.orchestrator = orchestrator
        self.isImplicit = isImplicit
    }

    /// The one workspace a runner without `workstreams` has in a repository.
    ///
    /// Its id is the repository's, so a board keyed by workspace still has a
    /// key, and a board notice from that runner (which names no workspace)
    /// still reaches it through `BoardNotice.touches`. Called Main because it
    /// is what Main was before it had a name.
    public static func implicit(repository: String) -> WorkspaceSummary {
        WorkspaceSummary(
            id: repository, repository: repository, name: "Main", taskPrefix: "",
            isMain: true, ordinal: 0, orchestrator: nil, isImplicit: true)
    }

    /// The workspace to name when reading this board, or nil for the whole
    /// repository — which is what an implicit workspace's board is.
    public var boardWorkspace: String? { isImplicit ? nil : id }

    enum CodingKeys: String, CodingKey {
        case id, repository, name, ordinal, orchestrator
        case taskPrefix = "task_prefix"
        case isMain = "is_main"
    }

    /// Hand-written for `WireTask`'s reason: a key added or dropped by a
    /// later runner must cost a field, never the whole fleet. Only `id` is
    /// required; a workspace with no id cannot be grouped under anything.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        repository = (try? c.decodeIfPresent(String.self, forKey: .repository)) ?? nil
        name = ((try? c.decodeIfPresent(String.self, forKey: .name)) ?? nil) ?? ""
        taskPrefix = ((try? c.decodeIfPresent(String.self, forKey: .taskPrefix)) ?? nil) ?? ""
        isMain = ((try? c.decodeIfPresent(Bool.self, forKey: .isMain)) ?? nil) ?? false
        ordinal = ((try? c.decodeIfPresent(Int.self, forKey: .ordinal)) ?? nil) ?? 0
        orchestrator = (try? c.decodeIfPresent(String.self, forKey: .orchestrator)) ?? nil
        isImplicit = false
    }
}

/// One workspace's place in the sidebar: its orchestrator and its worktrees.
public struct WorkspaceGroup: Identifiable, Equatable, Sendable {
    public let workspace: WorkspaceSummary
    /// The orchestrator's terminal id, or nil when none is running.
    public let orchestrator: String?
    /// Worktree ids, in the order the runner listed them.
    public let worktrees: [String]
    public var id: String { workspace.id }

    public init(workspace: WorkspaceSummary, orchestrator: String?, worktrees: [String]) {
        self.workspace = workspace
        self.orchestrator = orchestrator
        self.worktrees = worktrees
    }
}

/// One repository's workspaces, and the worktrees none of them owns.
public struct RepositoryGroups: Equatable, Sendable {
    /// The repository's id.
    public let repository: String
    /// Main first, then by the runner's ordinal. A workspace with no
    /// worktrees is still here: it still has a board and may have an
    /// orchestrator.
    public let workspaces: [WorkspaceGroup]
    /// Worktree ids no listed workspace owns, in runner order. Always empty
    /// for a runner without workspaces, whose implicit workspace owns all.
    public let unclaimed: [String]

    public init(repository: String, workspaces: [WorkspaceGroup], unclaimed: [String]) {
        self.repository = repository
        self.workspaces = workspaces
        self.unclaimed = unclaimed
    }
}

public enum WorkspaceGrouping {
    /// Group one repository's worktrees by the workspace that owns each.
    ///
    /// - `workspaces` are THIS repository's; the caller filters. Empty means
    ///   a runner without workspaces, and the result is one implicit
    ///   workspace holding every worktree, which is today's layout.
    /// - A worktree whose workspace is nil is unclaimed. So is one whose
    ///   workspace is not in `workspaces` — deleted since, or in another
    ///   repository — rather than being dropped from the sidebar.
    /// - `orchestrators` maps a workspace id to its orchestrator's terminal
    ///   id; a workspace missing from it falls back to its own
    ///   `orchestrator`.
    public static func group(
        repository: String, workspaces: [WorkspaceSummary],
        worktrees: [(id: String, workspace: String?)], orchestrators: [String: String]
    ) -> RepositoryGroups {
        guard !workspaces.isEmpty else {
            let only = WorkspaceSummary.implicit(repository: repository)
            return RepositoryGroups(
                repository: repository,
                workspaces: [
                    WorkspaceGroup(
                        workspace: only, orchestrator: nil, worktrees: worktrees.map(\.id))
                ],
                unclaimed: [])
        }
        let ordered = workspaces.enumerated().sorted { a, b in
            if a.element.isMain != b.element.isMain { return a.element.isMain }
            if a.element.ordinal != b.element.ordinal { return a.element.ordinal < b.element.ordinal }
            return a.offset < b.offset
        }.map(\.element)
        let known = Set(ordered.map(\.id))
        var owned: [String: [String]] = [:]
        var unclaimed: [String] = []
        for worktree in worktrees {
            if let workspace = worktree.workspace, known.contains(workspace) {
                owned[workspace, default: []].append(worktree.id)
            } else {
                unclaimed.append(worktree.id)
            }
        }
        return RepositoryGroups(
            repository: repository,
            workspaces: ordered.map { workspace in
                WorkspaceGroup(
                    workspace: workspace,
                    orchestrator: orchestrators[workspace.id] ?? workspace.orchestrator,
                    worktrees: owned[workspace.id] ?? [])
            },
            unclaimed: unclaimed)
    }
}

/// A board moved: what a `task` notice says, and which boards must re-read.
///
/// The FFI's line is `{"event": "task", "repository", "workspace",
/// "from_workspace", "actor"}` (`event_line` in `crates/client/src/ffi.rs`),
/// and the CLI's task event carries `workspace` too. A board keyed by
/// workspace re-reads on its own board's news and not on another's.
public struct BoardNotice: Equatable, Sendable {
    public let repository: String
    /// The board the task is on now. Nil from a runner without
    /// `workstreams`, which means every board in `repository` may have moved.
    public let workspace: String?
    /// The board it just left, set only on a move.
    public let fromWorkspace: String?
    public let actor: String?

    public init(
        repository: String, workspace: String?, fromWorkspace: String? = nil,
        actor: String? = nil
    ) {
        self.repository = repository
        self.workspace = workspace
        self.fromWorkspace = fromWorkspace
        self.actor = actor
    }

    /// Read a notice the client core queued, or nil when it is not board
    /// news. An empty string is read as absent, as a null is.
    public init?(notice: [String: Any]) {
        guard notice["event"] as? String == "task",
            let repository = notice["repository"] as? String, !repository.isEmpty
        else { return nil }
        func word(_ key: String) -> String? {
            (notice[key] as? String).flatMap { $0.isEmpty ? nil : $0 }
        }
        self.init(
            repository: repository, workspace: word("workspace"),
            fromWorkspace: word("from_workspace"), actor: word("actor"))
    }

    /// Whether `board` must be read again.
    ///
    /// With a workspace named: the board it is on now, and the board it left.
    /// With none — a runner without `workstreams` — every board in the
    /// repository, which on such a runner is its one implicit board.
    ///
    /// An implicit board is the whole repository's, so any task in the
    /// repository moves it, whichever workspace the notice names. That is a
    /// runner upgraded under a connected app, whose fleet has not listed the
    /// workspaces yet, or one whose list left a repository out: its notices
    /// name workspaces the boards are not keyed by.
    public func touches(_ board: WorkspaceSummary) -> Bool {
        if board.isImplicit { return board.repository == repository }
        guard let workspace else { return board.repository == repository }
        return board.id == workspace || board.id == fromWorkspace
    }
}

extension Fleet {
    /// This fleet grouped by repository and then by workspace, repositories
    /// in the order their first worktree appears.
    ///
    /// The phone's fleet names each worktree's repository by UUID, and so
    /// does each workspace, so they can be matched here. A repository with
    /// workspaces and no worktrees is listed after the rest, since it still
    /// has boards to show.
    func repositoryGroups() -> [RepositoryGroups] {
        var order: [String] = []
        var rows: [String: [(id: String, workspace: String?)]] = [:]
        for worktree in worktrees {
            let repository = worktree.repository ?? ""
            if rows[repository] == nil { order.append(repository) }
            rows[repository, default: []].append((worktree.id, worktree.workspace))
        }
        for workspace in workspaces ?? [] {
            let repository = workspace.repository ?? ""
            if rows[repository] == nil {
                order.append(repository)
                rows[repository] = []
            }
        }
        var orchestrators: [String: String] = [:]
        for worktree in worktrees {
            for terminal in worktree.terminals where terminal.isOrchestrator {
                if let workspace = terminal.workspace { orchestrators[workspace] = terminal.id }
            }
        }
        return order.map { repository in
            WorkspaceGrouping.group(
                repository: repository,
                workspaces: (workspaces ?? []).filter { $0.repository == repository },
                worktrees: rows[repository] ?? [], orchestrators: orchestrators)
        }
    }
}
