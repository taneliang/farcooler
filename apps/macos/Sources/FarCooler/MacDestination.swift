import AgentKit
import Foundation

// The Mac's side of `Destination` (ov-182, ov-183): what a window holds, as
// the resolver reads it, and the selection a resolved destination opens.
//
// A notification click and a relaunch both arrive as a `Destination` and wait
// in `DestinationOpener` for `DestinationResolver`, which answers when its
// runner has come up. Everything here is a value, so
// `MacDestinationTests` pins it without a window.

enum MacDestination {
    typealias World = DestinationResolver.World
    typealias Selection = ContentView.Selection

    /// One runner as a window holds it: `DaemonClient`'s answer, as values.
    struct Runner: Equatable, Sendable {
        /// `DaemonClient.target`; `""` is this Mac.
        var host: String
        /// Its `Host.runner_id`, once read.
        var runnerId: String?
        /// Connected, and its daemon build read: what `runnerId` says is so,
        /// and `task show` will be answered.
        var ready: Bool
        /// Its fleet has been read at least once. Until then nothing can be
        /// said to be gone from it.
        var loaded: Bool
    }

    // MARK: - What the window holds

    /// `runners`' fleets as the resolver reads them.
    ///
    /// A runner's workspaces are its own list, or, on one without
    /// `workstreams`, one implicit workspace per repository it has a worktree
    /// in, whose id is the repository's (`WorkspaceSummary.implicit`). Not
    /// read yet is `nil`, and that is different from read and empty.
    static func world(runners: [Runner], fleet: Fleet) -> World {
        World(
            seats: runners.map { runner in
                let worktrees = fleet.worktrees.filter { ($0.host ?? "") == runner.host }
                return World.Seat(
                    host: runner.host, runnerId: runner.runnerId, ready: runner.ready,
                    workspaces: runner.ready && runner.loaded ? workspaces(on: runner.host, worktrees: worktrees, fleet: fleet) : nil,
                    worktrees: runner.ready && runner.loaded
                        ? worktrees.map { worktree in
                            World.Worktree(
                                id: worktree.id, workspace: WorkspaceSelection.owner(of: worktree, in: fleet),
                                terminals: worktree.terminals.map {
                                    World.Terminal(id: $0.id, orchestrator: $0.isOrchestrator)
                                })
                        } : nil)
            })
    }

    private static func workspaces(on host: String, worktrees: [Worktree], fleet: Fleet) -> [World.Workspace] {
        if let listed = fleet.runnerWorkspaces[host] {
            return listed.map { World.Workspace(id: $0.id, orchestrator: $0.orchestrator != nil) }
        }
        var seen: Set<String> = []
        return worktrees.compactMap { $0.repositoryID }.filter { seen.insert($0).inserted }
            .map { World.Workspace(id: $0, orchestrator: false) }
    }

    // MARK: - A selection, kept

    /// Where `selection` is, with the finer parts a window holds beside it:
    /// the task's tab and chosen agent, and the pane the keyboard was in.
    /// Nil for nothing selected.
    static func destination(
        _ selection: Selection?, tab: TaskTab? = nil, agent: String? = nil, pane: String? = nil
    ) -> Destination? {
        switch selection {
        case nil:
            return nil
        case .needsYou:
            return .needsYou
        case .workspace(let host, let workspace, let focus):
            let runner = Destination.Runner(host: host)
            switch focus {
            case nil:
                return Destination(runner: runner, place: .workspace(workspace))
            case .task(let id):
                return Destination(
                    runner: runner, place: .task(workspace: workspace, task: .init(id: id)),
                    tab: tab.flatMap { Destination.Tab(rawValue: $0.rawValue) }, pane: pane, agent: agent)
            case .worktree(let id, let terminal):
                return Destination(
                    runner: runner, place: .worktree(id, workspace: workspace), pane: terminal ?? pane)
            case .history(let status):
                return Destination(runner: runner, place: .history(workspace: workspace, status: status.rawValue))
            }
        case .looseWorktree(let host, let worktree, let terminal):
            return Destination(
                runner: Destination.Runner(host: host), place: .worktree(worktree, workspace: nil), pane: terminal)
        }
    }

    /// `destination(_:tab:agent:pane:)` for what a window holds: the selection,
    /// the open task's chosen tab and agent, and, for an open worktree, the
    /// pane the keyboard is in.
    static func destination(
        _ selection: Selection?, tabs: TaskTabMemory, agents: [String: String], keyPane: PaneRef?
    ) -> Destination? {
        switch selection {
        case .workspace(_, _, .task(let id)?):
            return destination(selection, tab: tabs.chosen[id], agent: agents[id])
        case .workspace(_, _, .worktree(let id, nil)?), .looseWorktree(_, let id, nil):
            return destination(selection, pane: keyPane.flatMap { $0.worktree == id ? $0.terminal : nil })
        default:
            return destination(selection)
        }
    }

    // MARK: - A destination, opened

    /// The selection a resolved `destination` opens, or nil for one this
    /// fleet doesn't have after all. A pane lands where going to it always
    /// has: on its task when it has one, else its worktree under its owner
    /// (`WorkspaceSelection.landing`).
    static func selection(for destination: Destination, in fleet: Fleet) -> Selection? {
        let host = destination.runner.host ?? ""
        switch destination.place {
        case .needsYou:
            return .needsYou
        case .terminal:
            return nil
        case .workspace(let id), .orchestrator(let id):
            return .workspace(host: host, workspace: id, focus: nil)
        case .history(let id, let status):
            return TaskStatus(rawValue: status).map { .workspace(host: host, workspace: id, focus: .history($0)) }
        case .task(let workspace, let task):
            guard let workspace, let id = task.id else { return nil }
            return .workspace(host: host, workspace: workspace, focus: .task(id))
        case .worktree(let id, _):
            guard let worktree = WorkspaceSelection.worktree(host: host, id: id, in: fleet) else { return nil }
            return WorkspaceSelection.landing(in: worktree, terminal: destination.pane, fleet: fleet)
        }
    }

    /// What a window holds, as the resolver reads it.
    @MainActor
    static func world(of store: FleetStore) -> World {
        world(
            runners: store.clients.values.sorted { $0.target < $1.target }.map { Runner($0) },
            fleet: store.fleet)
    }

    /// What opening a resolved destination does, as values; `ContentView.land` does it.
    struct Landing: Equatable {
        /// A task a click opens as the navigator does (`openTask`).
        var openTask: OpenTask?
        /// What to select otherwise, and the pane the keyboard goes to.
        var selection: Selection?
        var pane: PaneRef?
        /// The task that ends up open, whose tab and chosen agent come back with it.
        var taskID: String?
        var tab: TaskTab?
        var agent: String?

        struct OpenTask: Equatable {
            var id: String
            var host: String
            var workspace: String
        }
    }

    /// Where opening `destination` lands. A click on a task goes through
    /// `openTask`, the palette's way; anything else is the selection
    /// `selection(for:in:)` says, with the keyboard in the pane it names.
    static func landing(_ destination: Destination, click: Bool, in fleet: Fleet) -> Landing {
        var landing = Landing()
        if click, case .task(let workspace?, let ref) = destination.place, let id = ref.id {
            landing.openTask = .init(id: id, host: destination.runner.host ?? "", workspace: workspace)
            landing.taskID = id
        } else if let next = selection(for: destination, in: fleet) {
            landing.selection = next
            landing.pane = pane(of: destination)
            if case .workspace(_, _, .task(let id)?) = next { landing.taskID = id }
        }
        if landing.taskID != nil {
            landing.tab = destination.tab.flatMap { TaskTab(rawValue: $0.rawValue) }
            landing.agent = destination.agent
        }
        return landing
    }

    /// The pane the keyboard goes to once `destination` is open, when it
    /// names one: only a worktree's, since a task's agent is `agent`.
    static func pane(of destination: Destination) -> PaneRef? {
        guard case .worktree(let worktree, _) = destination.place, let pane = destination.pane else { return nil }
        return PaneRef(host: destination.runner.host ?? "", worktree: worktree, terminal: pane)
    }
}

extension Destination {
    /// A task notice's click, as a destination: its runner by the id the
    /// notice carries (or the one its id spells) and by the target a local
    /// post names, and its task by key and repository.
    init(notice: TaskNotice, target: String?, repository: String?) {
        let runner = notice.runner ?? notice.noticeId.flatMap(Destination.parse(noticeId:))?.runner
        self.init(
            runner: Runner(host: target, id: runner?.lowercased()),
            place: .task(workspace: nil, task: TaskRef(key: notice.key, repository: repository)),
            question: notice.event == .decision)
    }
}
