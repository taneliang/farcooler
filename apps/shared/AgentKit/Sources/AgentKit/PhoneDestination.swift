import Foundation

/// The phone's side of `Destination` (ov-182, ov-183): what its runners hold,
/// as the resolver reads it, and the screens a resolved destination opens.
///
/// A tapped notification, a card's link and a relaunch all arrive as a
/// `Destination` and wait in `PhoneRoot` for `DestinationResolver`, which
/// answers when its runner has come up and says where it ended up; `link`
/// then lays out the stack for that place, the way `Fleet.phoneLink` always
/// did for a terminal.
enum PhoneDestination {
    /// One runner as the phone holds it, in plain values, so a test builds
    /// the same thing a `Connection` is read into.
    struct Source {
        /// The phone's own handle (`Host.id`, as a string), which a saved
        /// stack names it by.
        var host: String
        /// Its `Host.runner_id`, once its daemon build has been read.
        var runnerId: String?
        /// Connected, with its fleet and daemon build read: `runnerId` is
        /// what it says.
        var ready: Bool
        /// Paired, and nothing is connecting it (the "Connect every runner
        /// at once" setting is off and it isn't the selected one).
        var idle: Bool
        var fleet: Fleet?
        /// Its boards as listed (`Connection.boardList`), empty until its
        /// repositories are read.
        var boardList: [WorkspaceSummary] = []
        /// Its boards read so far, by workspace id.
        var boards: [String: TaskBoardModel] = [:]
    }

    /// What the phone holds now, for `DestinationResolver`.
    static func world(_ sources: [Source], last: PhoneWorkspace?) -> DestinationResolver.World {
        DestinationResolver.World(
            seats: sources.map(seat),
            lastWorkspace: last.map { DestinationResolver.World.Last(host: $0.runner, workspace: $0.workspace) })
    }

    private static func seat(_ source: Source) -> DestinationResolver.World.Seat {
        let fleet = source.ready ? source.fleet : nil
        return DestinationResolver.World.Seat(
            host: source.host, runnerId: source.runnerId, ready: source.ready,
            idle: source.idle,
            // Not read is not empty: no boards listed yet is no claim that
            // none exist.
            workspaces: fleet == nil || source.boardList.isEmpty
                ? nil
                : source.boardList.map {
                    DestinationResolver.World.Workspace(id: $0.id, orchestrator: $0.orchestrator != nil)
                },
            worktrees: fleet.map { fleet in
                fleet.worktrees.map { worktree in
                    let orchestrator = worktree.terminals.first(where: \.isOrchestrator)
                    return DestinationResolver.World.Worktree(
                        id: worktree.id,
                        workspace: Self.nonEmpty(worktree.workspace) ?? Self.nonEmpty(orchestrator?.workspace)
                            ?? (fleet.workspaces == nil ? worktree.repository : nil),
                        terminals: worktree.terminals.map {
                            DestinationResolver.World.Terminal(id: $0.id, orchestrator: $0.isOrchestrator)
                        })
                }
            },
            boards: source.boards.mapValues { board in
                board.rows.map { DestinationResolver.World.Task(id: $0.id, key: $0.key) }
            })
    }

    private static func nonEmpty(_ text: String?) -> String? {
        text.flatMap { $0.isEmpty ? nil : $0 }
    }

    /// The screens a resolved destination opens, and the segment a workspace
    /// at the top of them shows. An empty stack is Needs You.
    ///
    /// `fleet` is the destination's runner's, for the task a pane belongs to
    /// and whether it's an orchestrator's, which is `Fleet.phoneLink`'s to
    /// say.
    static func link(for destination: Destination, fleet: Fleet?) -> PhoneLink {
        guard let host = destination.runner.host else { return PhoneLink(stack: [], segment: nil) }
        func workspace(_ id: String) -> PhoneWorkspace { PhoneWorkspace(runner: host, workspace: id) }
        switch destination.place {
        case .needsYou, .terminal:
            return PhoneLink(stack: [], segment: nil)
        case .workspace(let id):
            return PhoneLink(stack: [.workspace(workspace(id))], segment: destination.segment.map(segment))
        case .orchestrator(let id):
            return PhoneLink(stack: [.workspace(workspace(id))], segment: .orchestrator)
        case .history(let id, let status):
            return PhoneLink(stack: [.workspace(workspace(id)), .history(workspace(id), status: status)], segment: nil)
        case .task(let id, let ref):
            guard let id, let task = ref.id else { return PhoneLink(stack: [], segment: nil) }
            return PhoneLink(stack: [.workspace(workspace(id)), .task(workspace(id), task: task)], segment: nil)
        case .worktree(let id, let owner):
            if let pane = destination.pane, let link = fleet?.phoneLink(toTerminal: pane, runner: host) {
                return link
            }
            var stack: [PhoneRoute] = owner.map { [.workspace(workspace($0))] } ?? []
            stack.append(.worktree(runner: host, worktree: id, landing: destination.pane.map { .terminal($0) } ?? .resume))
            return PhoneLink(stack: stack, segment: nil)
        }
    }

    static func segment(_ segment: Destination.Segment) -> WorkspaceSegment {
        switch segment {
        case .orchestrator: .orchestrator
        case .board: .board
        case .worktrees: .worktrees
        }
    }
}
