import Foundation

// The phone's Board rows: one per workspace that has a board, in its runner's
// section of the shell overview.
//
// The Mac's old Fleet sidebar had this row from card -19 until ov-178 retired
// the sidebar, and it drew the same two
// numbers: the tasks waiting on a decision, amber, and a quiet count of the
// tasks an agent is on. The rules for both are AgentKit's already —
// `TaskBoardModel.waitingOnYou`, `tasksWithLiveAgents`, `TaskAgentLink` — and
// what this file adds is which boards get a row at all, which boards a sweep
// reads, and which boards a notice moves: rules a view body or a connection
// would otherwise decide and no suite would read.
//
// A board is a WORKSPACE's now. A repository has one board per workspace, and
// a runner without `workstreams` has one implicit workspace per repository
// (`WorkspaceSummary.implicit`), whose board is the whole repository's — which
// is exactly the board the phone had before workspaces existed.

/// One workspace's Board row.
public struct RunnerBoardRow: Equatable, Sendable, Identifiable {
    /// The board: which workspace, and how to read it
    /// (`WorkspaceSummary.boardWorkspace`).
    public var workspace: WorkspaceSummary
    /// What to call it: the workspace's name, or for an implicit workspace the
    /// repository's display name, which is what the row said before.
    public var name: String
    /// Tasks in Needs Decision. Drawn in amber, and not at all at zero.
    public var decisions: Int
    /// Tasks at least one live agent is on. Drawn quietly, and not at all at
    /// zero — which is also what a runner that cannot say reads as.
    public var agents: Int

    /// The workspace's id, which is what the boards are keyed by.
    public var id: String { workspace.id }

    /// The repository's uuid, which is what `task.list` takes beside the
    /// workspace.
    public var repository: String { workspace.repository ?? workspace.id }

    public init(workspace: WorkspaceSummary, name: String, decisions: Int, agents: Int) {
        self.workspace = workspace
        self.name = name
        self.decisions = decisions
        self.agents = agents
    }

    /// A row for a repository's one implicit board: a runner without
    /// workspaces, or a fixture.
    public init(repository: String, name: String, decisions: Int, agents: Int) {
        self.init(
            workspace: .implicit(repository: repository), name: name, decisions: decisions,
            agents: agents)
    }

    /// What VoiceOver reads after the row's name: the two counts in words,
    /// or nil when neither says anything. The Mac's tooltips, joined.
    public var spoken: String? {
        let parts = [
            TaskBoardModel.decisionsHelp(decisions), TaskBoardModel.agentsHelp(agents),
        ].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }
}

public enum RunnerBoards {
    /// Every board a runner keeps, in the order its sections draw them: each
    /// repository's workspaces, Main first and then by ordinal, or its one
    /// implicit workspace where the runner named none.
    ///
    /// What a sweep reads — a link coming up, a reconnect, a `resync` — and so
    /// what an open board relies on to be read again after a dropped link
    /// whatever notices were lost with it. A sweep that walked repositories
    /// while the boards were keyed by workspace would read the whole
    /// repository into a key no view looks at, and leave every workspace's
    /// board showing what the previous link read.
    ///
    /// - `repositories` are the runner's, in its order. A repository no
    ///   workspace names is implicit: a runner without `workstreams`
    ///   (`workspaces` nil), or one whose list left it out.
    /// - A workspace in a repository missing from `repositories` still has a
    ///   board, and comes after the rest: `repositories` is a separate read
    ///   that can lag the fleet.
    public static func boards(
        repositories: [String], workspaces: [WorkspaceSummary]?
    ) -> [WorkspaceSummary] {
        let all = workspaces ?? []
        var order = repositories
        for workspace in all {
            if let repository = workspace.repository, !order.contains(repository) {
                order.append(repository)
            }
        }
        return order.flatMap { repository in
            WorkspaceGrouping.group(
                repository: repository,
                workspaces: all.filter { $0.repository == repository },
                worktrees: [], orchestrators: [:]
            ).workspaces.map(\.workspace)
        }
    }

    /// The boards a notice moved, of `boards`, which is what a connection
    /// holds — plus the board it names when that board is not among them yet.
    ///
    /// `BoardNotice.touches` decides. The addition is for a workspace the
    /// fleet has not listed yet: made by the CLI a moment ago, its first task
    /// filed before the fleet read that would have listed it. Reading its
    /// board now means its row is there the moment the fleet names it, rather
    /// than after the next sweep, which on a quiet link is the next reconnect.
    public static func touched(
        by notice: BoardNotice, among boards: [WorkspaceSummary]
    ) -> [WorkspaceSummary] {
        var found = boards.filter { notice.touches($0) }
        for named in [notice.workspace, notice.fromWorkspace].compactMap({ $0 })
        where !found.contains(where: { $0.id == named }) {
            found.append(
                WorkspaceSummary(
                    id: named, name: "", taskPrefix: "", isMain: false, ordinal: 0,
                    repository: notice.repository))
        }
        return found
    }

    /// The Board rows for one runner, in the order `boards` lists them.
    ///
    /// - A runner that does not advertise `tasks` has no board, and gets no
    ///   rows. Nor does one no link has asked yet (`build` and
    ///   `lastKnownBuild` both nil): a row that appeared and then vanished on
    ///   the first answer would move every card under it.
    /// - A workspace gets a row whenever it exists, its board read or not
    ///   and empty or not (ov-56): the row is the way onto the board, and a
    ///   workspace made a moment ago has nothing on it yet. The board draws
    ///   the empty and unread states. Android does the same (8819fc06).
    /// - An implicit board, a repository's on a runner without workspaces,
    ///   gets a row the same way, empty or unread (ov-55, spec §5 and §8).
    ///   It used to need something on it, since nobody made a workspace
    ///   there; but the workspace view treats it as a workspace, and an
    ///   empty board with no row hid its Worktrees view too, where New
    ///   Worktree… is.
    /// - `agents` is counted only where `TaskAgentLink.speaksOfAgents` says
    ///   the runner can be believed about its panes. Anywhere else it is 0,
    ///   which draws nothing: "can't say", not "none".
    /// - A workspace's row is called by the workspace's name; an implicit
    ///   one's by its repository's, from `names`, as the row always was.
    ///
    /// `models` holds the last good read of each board, by workspace id. It is
    /// kept through a reconnect on purpose, like the fleet: the decisions are
    /// what the runner last said, and a row that blinked out for every
    /// dropped link would be a row nobody could find twice.
    ///
    /// `build` is what THIS link has read, nil from the moment a link comes up
    /// until its `host` answers. `lastKnownBuild` is the last any link read,
    /// kept through a reconnect, and it is what keeps the rows drawn in that
    /// gap: a row that vanished for one round trip after every Wi-Fi blink
    /// would move every card under it up and back down. Agents are counted
    /// only against `build` — the fresh one — so in the gap the rows stay and
    /// say nothing about agents.
    public static func rows<P: TaskBoardPane>(
        boards: [WorkspaceSummary],
        names: [String: String],
        models: [String: TaskBoardModel],
        panes: [P],
        build: DaemonBuild?,
        lastKnownBuild: DaemonBuild? = nil,
        needsYou: [NeedsYouItem]? = nil,
        connected: Bool
    ) -> [RunnerBoardRow] {
        guard (build ?? lastKnownBuild)?.can("tasks") == true else { return [] }
        let served = (build ?? lastKnownBuild)?.can("needs_you") == true
        let speaks = TaskAgentLink.speaksOfAgents(connected: connected, build: build)
        return boards.map { workspace in
            let board = models[workspace.id]
            let repository = workspace.repository ?? workspace.id
            return RunnerBoardRow(
                workspace: workspace,
                name: workspace.isImplicit ? (names[repository] ?? workspace.name) : workspace.name,
                decisions: waiting(
                    columnCount: board?.waitingOnYou ?? 0,
                    decisions: decisions(for: workspace, in: needsYou ?? []),
                    listRead: needsYou != nil, listServed: served),
                agents: speaks ? (board?.tasksWithLiveAgents(in: panes) ?? 0) : 0)
        }
    }

    /// The decision items `workspace` has, out of one runner's needs-you list:
    /// its items that are a decision, alone or beside an ask. Not the Needs
    /// Decision column's rows, which an answer leaves as they were (spec
    /// §2.2: an answer doesn't move status). An implicit workspace counts its
    /// repository's items that name no workspace.
    ///
    /// `items` are ONE runner's: the phone's connection holds a list per
    /// runner, and a merged list would need the runner matched too.
    public static func decisions(for workspace: WorkspaceSummary, in items: [NeedsYouItem]) -> Int {
        items.filter { item in
            guard item.kind == .decision || item.also.contains(.decision) else { return false }
            if workspace.isImplicit {
                return item.workspaceID == nil && item.repositoryID == workspace.id
            }
            return item.workspaceID == workspace.id
        }.count
    }

    /// What a board says is waiting on the person: the decision items once
    /// the runner's list is read, and the Needs Decision column's count
    /// until then, and always on a runner that serves no list. An unread
    /// list is not a list with nothing in it. The Mac's
    /// `WorkspaceCounts.waiting`, and the same rule.
    public static func waiting(
        columnCount: Int, decisions: Int, listRead: Bool, listServed: Bool
    ) -> Int {
        listRead && listServed ? decisions : columnCount
    }

    /// What a workspace's board screen says is waiting, from the board it
    /// holds and one runner's needs-you reading: `waiting` over the board's
    /// Needs Decision column and `decisions`. Zero for a board not read
    /// yet. `listRead` is a list the runner itself served; a reading derived
    /// from an older runner's fleet is not one, and such a runner's `build`
    /// doesn't advertise `needs_you` either, which is the guard that counts.
    public static func waiting(
        on board: TaskBoardModel?, in workspace: WorkspaceSummary, items: [NeedsYouItem],
        listRead: Bool, build: DaemonBuild?
    ) -> Int {
        guard let board else { return 0 }
        return waiting(
            columnCount: board.waitingOnYou, decisions: decisions(for: workspace, in: items),
            listRead: listRead, listServed: build?.can("needs_you") == true)
    }

    /// The Board rows of a runner without workspaces: one per repository, in
    /// the order the runner lists them, each the repository's implicit board.
    /// `rows(boards:…)` over `WorkspaceSummary.implicit`, for a caller that
    /// holds repositories and not workspaces.
    public static func rows<P: TaskBoardPane>(
        repositories: [(id: String, name: String)],
        boards: [String: TaskBoardModel],
        panes: [P],
        build: DaemonBuild?,
        lastKnownBuild: DaemonBuild? = nil,
        needsYou: [NeedsYouItem]? = nil,
        connected: Bool
    ) -> [RunnerBoardRow] {
        rows(
            boards: repositories.map { WorkspaceSummary.implicit(repository: $0.id) },
            names: Dictionary(repositories.map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a }),
            models: boards, panes: panes, build: build, lastKnownBuild: lastKnownBuild,
            needsYou: needsYou, connected: connected)
    }
}

/// Whether a link still owes its boards a read, and which link a sweep is for.
///
/// A new link clears it; a sweep that read the repositories sets it. When the
/// runner's build lands on a link whose sweep was refused for want of it,
/// the boards are read then: a sweep started before the build was known
/// refused every board (it cannot know the runner keeps one), and without
/// this the overview's rows would go on showing what the previous link read
/// until some board happened to move. The first `host` read on a new link
/// failing is exactly when that happens — a reconnect over a link still
/// coming up.
///
/// Only a refused sweep is owed. Every link-up reads the build and then
/// sweeps, so a build landing with no sweep refused yet has one on its way;
/// sweeping for it too read every board twice on every connect.
///
/// And a sweep belongs to its link: `link` is bumped by each new one, and a
/// sweep that finds itself no longer `isCurrent` stops, rather than reading
/// on over the new link beside that link's own sweep.
public struct BoardSweep: Equatable, Sendable {
    public private(set) var sweptOnThisLink = false
    public private(set) var refusedOnThisLink = false
    public private(set) var link = 0

    public init() {}

    /// A new link: its boards have not been read on it, and a sweep still
    /// going from the last one is no longer current.
    public mutating func linkCameUp() {
        sweptOnThisLink = false
        refusedOnThisLink = false
        link += 1
    }

    /// This link's boards were read.
    public mutating func swept() { sweptOnThisLink = true }

    /// A sweep on this link found no build to say whether the runner keeps a
    /// board, and read nothing.
    public mutating func refused() { refusedOnThisLink = true }

    /// Whether the build landing now should start a sweep.
    public var owedWhenBuildLands: Bool { refusedOnThisLink && !sweptOnThisLink }

    /// Whether a sweep started on `link` is still this link's.
    public func isCurrent(_ link: Int) -> Bool { link == self.link }
}

extension TaskBoardModel {
    /// The board's sections: every status in `order`, Needs Decision first,
    /// each with its rows and count, the empty ones included (owner decision
    /// 3, spec §5).
    ///
    /// What the list form draws. An empty status is a collapsed header
    /// reading "Backlog 0", not a gap: a list that dropped it said nothing
    /// about what isn't there, and moved every heading under it when a task
    /// was filed. Built over `order` rather than `columns`, so a board read
    /// with some columns or none (`.empty`) still has all seven.
    public var sections: [TaskBoardColumn] {
        TaskBoardModel.order.map { status in
            TaskBoardColumn(
                status: status, rows: columns.first { $0.status == status }?.rows ?? [])
        }
    }
}

extension TaskBoardColumn {
    /// How many tasks are in this status: what a section's header shows,
    /// 0 included.
    public var count: Int { rows.count }
}
