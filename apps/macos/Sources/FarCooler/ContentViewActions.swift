import AgentKit
import SwiftUI

/// The clicks that reach a runner, and where their results go: `act` files a
/// result under the action and its target (`ActionOutcomes`), and the
/// terminal and orchestrator actions run through it. Split out of
/// `ContentView.swift`, which has a size ceiling (ov-135).
extension ContentView {
    /// Route a click to the runner a worktree is on, refusing it first, and
    /// file its result under `verb` and `target` (`ActionOutcomes`).
    ///
    /// Checked here rather than at each call site — see `FleetStore.refusal(for:)`
    /// for why. On refusal, `fallback` is handed back, nothing is called, and
    /// the refusal is the result. Otherwise the result is what the commands
    /// `body` ran said (`ActionReport`): the first failure in the app's
    /// words, or nothing, which also takes down this action's earlier
    /// failure on the same target. `target` defaults to the worktree's id,
    /// `subject` (how the sentence names it) to its name.
    @discardableResult
    func act<T>(
        _ verb: ActionVerb, on ws: Worktree, target: String? = nil, subject: String? = nil,
        default fallback: T, _ body: (DaemonClient) async -> T
    ) async -> T {
        let host = ws.host ?? ""
        let key = ActionKey(verb: verb, host: host, target: target ?? ws.id)
        let named = subject ?? Self.quoted(ws)
        if let why = store.refusalSentence(for: host) {
            outcomes.settle(key, failure: "\(verb.lead(named)) \(why)")
            return fallback
        }
        guard let client = store.client(for: ws) else { return fallback }
        return await outcomes.perform(key, subject: named, on: client, body)
    }

    func act(
        _ verb: ActionVerb, on ws: Worktree, target: String? = nil, subject: String? = nil,
        _ body: (DaemonClient) async -> Void
    ) async {
        await act(verb, on: ws, target: target, subject: subject, default: ()) { client in await body(client) }
    }

    /// A worktree as a sentence names it: its task, in quotes.
    static func quoted(_ ws: Worktree) -> String { "“\(WorktreeName.display(ws.task))”" }

    /// A terminal as a sentence names it: its label, in quotes.
    static func quoted(_ terminal: Terminal) -> String { "“\(terminal.label)”" }

    /// File the result of an action whose client call words its own
    /// failure: the sentence, or nil for one that worked. Same lifetime as
    /// `act`'s results, so navigation leaves it.
    func fileResult(_ verb: ActionVerb, host: String, target: String, _ failure: String?) {
        outcomes.settle(ActionKey(verb: verb, host: host, target: target), failure: failure)
    }


    // MARK: - Terminal actions

    func run(_ action: TerminalAction, on term: Terminal, in worktree: Worktree) async {
        switch action {
        case .restart:
            await act(.restart, on: worktree, target: term.id, subject: Self.quoted(term)) { c in
                await c.restart(terminal: term.short)
            }
        case .dismissLost:
            await act(.dismissLost, on: worktree, target: term.id, subject: Self.quoted(term)) { c in
                await c.dismissLost(term)
            }
        case .stop:
            await act(.stop, on: worktree, target: term.id, subject: Self.quoted(term)) { c in
                await c.stop(terminal: term.short)
            }
        case .close:
            // As ⌘W: stop it, then remove the record, which is one action. The
            // layout is read again because the runner publishes none when a
            // pane closes (checklist O1).
            await act(.close, on: worktree, target: term.id, subject: Self.quoted(term)) { c in
                await c.stop(terminal: term.short)
                await c.removeTerminal(term.short)
            }
            await store.client(for: worktree)?.refreshLayout(worktree)
        case .rename:
            renaming = RenamingTerminal(terminal: term, worktree: worktree)
        case .openInBrowser:
            if let url = TerminalPorts.browserURL(for: term, host: worktree.host ?? "") {
                NSWorkspace.shared.open(url)
            }
        case .useAsOrchestrator: useAsOrchestrator(BoardPane(terminal: term, worktree: worktree))
        case .stopBeingOrchestrator: await stepDown(BoardPane(terminal: term, worktree: worktree))
        }
    }

    /// Use as Orchestrator: make `pane`, already running, its workspace's
    /// orchestrator. Asks first when that would replace one, naming it.
    func useAsOrchestrator(_ pane: BoardPane) {
        let host = pane.worktree.host ?? ""
        if let why = store.refusalSentence(for: host) {
            fileResult(.setRole, host: host, target: pane.terminal.id,
                 "\(ActionVerb.setRole.lead(Self.quoted(pane.terminal))) \(why)")
            return
        }
        guard let id = pane.terminal.workspace,
            let workspace = store.fleet.runnerWorkspaces[host]?.first(where: { $0.id == id })
        else {
            fileResult(.setRole, host: host, target: pane.terminal.id, OrchestratorAdoption.refusal(
                "code: invalid-argument\nwhat: workspace", terminal: pane.terminal.label, workspace: ""))
            return
        }
        if let old = OrchestratorAdoption.replacing(pane, in: workspace, host: host, fleet: store.fleet) {
            adoptionPending = OrchestratorAdoptionPending(host: host, workspace: workspace, pane: pane, old: old)
        } else {
            Task { await adopt(pane, in: workspace, host: host, replacing: nil) }
        }
    }

    /// Set the roles: `old` steps down first, since the runner allows one
    /// live orchestrator a workspace, then `pane` takes the seat, and every
    /// other pane in its window is moved to a window of its own. If the
    /// runner refuses `pane`, `old` is put back, so a refusal never leaves
    /// the workspace with none. The column follows on the refresh.
    func adopt(_ pane: BoardPane, in workspace: WorkspaceSummary, host: String, replacing old: BoardPane?) async {
        guard let client = store.clients[host] else { return }
        if let old {
            let (refused, message) = await client.setRole(old.terminal, to: OrchestratorAdoption.steppedDown(old.terminal))
            if refused {
                fileResult(.setRole, host: host, target: pane.terminal.id,
                     OrchestratorAdoption.refusal(message, terminal: old.terminal.label, workspace: workspace.name))
                return
            }
        }
        let (refused, message) = await client.setRole(pane.terminal, to: "orchestrator")
        guard refused else {
            fileResult(.setRole, host: host, target: pane.terminal.id, nil)
            // Adopted: what shares its window moves out, the orchestrator it
            // replaced included, so the column draws it alone (ov-78). The
            // adopting is the ask.
            for other in WorkspaceScreen.movedOnAdopting(
                pane, replacing: old?.terminal.id, layouts: client.layouts[pane.worktree.id])
            {
                await moveOutOfOrchestratorWindow(other, in: pane.worktree)
            }
            return
        }
        fileResult(.setRole, host: host, target: pane.terminal.id,
             OrchestratorAdoption.refusal(message, terminal: pane.terminal.label, workspace: workspace.name))
        if let old { _ = await client.setRole(old.terminal, to: "orchestrator") }
    }

    /// Stop Being Orchestrator: `pane` goes back to what it would have been
    /// made as (`OrchestratorAdoption.steppedDown`), and keeps running.
    func stepDown(_ pane: BoardPane) async {
        guard let client = store.client(for: pane.worktree) else { return }
        let workspace = pane.terminal.workspace.flatMap { id in
            store.fleet.runnerWorkspaces[pane.worktree.host ?? ""]?.first { $0.id == id }
        }
        let (refused, message) = await client.setRole(pane.terminal, to: OrchestratorAdoption.steppedDown(pane.terminal))
        fileResult(.setRole, host: pane.worktree.host ?? "", target: pane.terminal.id, refused ? OrchestratorAdoption.refusal(
            message, terminal: pane.terminal.label, workspace: workspace?.name ?? "The workspace") : nil)
    }
}
