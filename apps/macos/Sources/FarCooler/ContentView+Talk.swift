import AgentKit
import SwiftUI

/// A member's Talk on the Mac (ov-526): their own rep where the orchestrator
/// would be, started first when it isn't running. The runner refuses anyone
/// but the lead and the owner a composer on the orchestrator (`not_lead`), so
/// nothing here draws one for them.
extension ContentView {
    /// Whom `workspace`'s Talk is on `host`: the orchestrator for this
    /// runner's owner and its lead, a rep for any other member.
    func talkSeat(host: String, workspace: WorkspaceSummary) -> TalkSeat {
        guard let client = store.clients[host] else { return .orchestrator }
        return client.talk.seat(for: workspace, build: client.daemonBuild)
    }

    /// What each orchestrator surface does here (`TalkSeat.surfaces`): the
    /// one place the column, the menu, Ask, the rulings and Use as
    /// Orchestrator ask, each by its own field.
    func surfaces(host: String, workspace: WorkspaceSummary) -> TalkSeat.Surfaces {
        talkSeat(host: host, workspace: workspace).surfaces
    }

    /// The jump bar's word and pane for a member's place, or nil where it's
    /// the orchestrator's.
    func talkPlace(host: String, workspace: WorkspaceSummary) -> (title: String, pane: BoardPane?)? {
        let seat = talkSeat(host: host, workspace: workspace)
        guard !seat.surfaces.columnIsOrchestrator else { return nil }
        guard case let .rep(_, _, existing) = seat else { return (seat.title, nil) }
        return (seat.title, repPane(host: host, existing: existing))
    }

    /// A member's rep pane, as the fleet has it.
    func repPane(host: String, existing: RunnerPeople.Rep?) -> BoardPane? {
        guard let existing else { return nil }
        for worktree in store.fleet.worktrees where (worktree.host ?? "") == host {
            if let terminal = worktree.terminals.first(where: { $0.id == existing.terminal }) {
                return BoardPane(terminal: terminal, worktree: worktree)
            }
        }
        return nil
    }

    /// The rep's column: its pane while it runs, else its first screen.
    @ViewBuilder
    func talkColumn(host: String, workspace: WorkspaceSummary, seat: TalkSeat, onScreen: Bool) -> some View {
        let client = store.clients[host]
        let key = "\(host)|\(workspace.id)"
        let existing: RunnerPeople.Rep? = { if case let .rep(_, _, e) = seat { return e } else { return nil } }()
        let pane = repPane(host: host, existing: existing)
        let state = RepColumn.state(
            seat: seat, pane: pane?.terminal, isStarting: startingReps.contains(key))
        Group {
            if state == .live, let pane {
                bareTerminal(pane, keyboard: onScreen)
            } else {
                RepPlaceholder(
                    state: state,
                    intro: client?.talk.people?.repIntro ?? [],
                    refusal: repRefusals[key],
                    canAct: store.refusal(for: host) == nil,
                    onStart: { startRep(host: host, workspace: workspace) },
                    onTryAgain: { if let client { Task { await client.talk.load(client, force: true) } } })
            }
        }
        .task(id: "\(host)|\(client?.daemonBuild?.version ?? "")") {
            if let client { await client.talk.load(client) }
        }
    }

    /// Start (or start again) the rep for this Mac's person in `workspace`.
    /// "Starting Your Rep…" shows until the runner answers; a refusal is
    /// said in the column, in this app's words.
    func startRep(host: String, workspace: WorkspaceSummary) {
        guard let client = store.clients[host], case let .rep(handle, _, _) = talkSeat(host: host, workspace: workspace)
        else { return }
        let key = "\(host)|\(workspace.id)"
        guard !startingReps.contains(key) else { return }
        startingReps.insert(key)
        repRefusals[key] = nil
        Task {
            repRefusals[key] = await client.talk.start(client, workspace: workspace, handle: handle)
            startingReps.remove(key)
        }
    }

    /// Choosing Talk starts a rep that isn't running: the ask.
    func startRepIfNotRunning(host: String, workspace: WorkspaceSummary) {
        let seat = talkSeat(host: host, workspace: workspace)
        guard case let .rep(_, _, existing) = seat else { return }
        let key = "\(host)|\(workspace.id)"
        let state = RepColumn.state(
            seat: seat, pane: repPane(host: host, existing: existing)?.terminal, isStarting: startingReps.contains(key))
        if RepColumn.startsOnTalk(state, canAct: store.refusal(for: host) == nil) { startRep(host: host, workspace: workspace) }
    }

    /// The navigator's row for a member: "Your Rep", its state as the
    /// orchestrator's row says it, and Start Your Rep while it isn't running.
    func navigatorRep(host: String, workspace: WorkspaceSummary, seat: TalkSeat) -> NavigatorOrchestrator {
        let existing: RunnerPeople.Rep? = { if case let .rep(_, _, e) = seat { return e } else { return nil } }()
        let pane = repPane(host: host, existing: existing)
        let key = "\(host)|\(workspace.id)"
        let column = RepColumn.state(seat: seat, pane: pane?.terminal, isStarting: startingReps.contains(key))
        var state = OrchestratorRow.state(seat: pane)
        switch column {
        case .starting: state = .starting
        case .none, .reading, .unknown, .neither: state = .none
        default: break
        }
        let canStart = RepColumn.startsOnTalk(column, canAct: store.refusal(for: host) == nil)
        return NavigatorOrchestrator(
            state: state,
            agent: pane.map { OrchestratorMenu.agentName($0) } ?? nil,
            status: pane?.terminal.status,
            nowDoing: OrchestratorRow.nowDoing(pane?.terminal, state: state),
            onSelect: {
                selectOrchestrator(keyboard: .board)
            },
            title: seat.title,
            noneTitle: seat.title,
            onStartRep: canStart && column == .none ? { startRep(host: host, workspace: workspace) } : nil)
    }
}
