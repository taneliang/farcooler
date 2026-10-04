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

    /// Close a terminal, asking first when an agent is mid-turn (ov-161).
    ///
    /// ⌘W, Close Pane and the context menu's Close Terminal all come here. An
    /// agent that is working or waiting on you has something to lose, and the
    /// phones ask about it (`ShellClose`); a shell, or an agent doing nothing,
    /// closes at once, since asking about it is a tax on the harmless case.
    func requestClose(_ terminal: Terminal, in worktree: Worktree) {
        if let question = CloseTerminalGuard.question(for: terminal, at: Date()) {
            closePending = CloseTerminalPending(worktree: worktree, terminal: terminal, question: question)
        } else {
            Task { await close(terminal, in: worktree) }
        }
    }

    /// Stop, then remove the record. Closing a terminal should leave nothing
    /// behind. One action: a Close whose stop was refused fails its remove
    /// too, and that is one thing that didn't happen, not two.
    func close(_ terminal: Terminal, in worktree: Worktree) async {
        await act(.close, on: worktree, target: terminal.id, subject: Self.quoted(terminal)) { c in
            await c.stop(terminal: terminal.short)
            await c.removeTerminal(terminal.short)
        }
        // The runner publishes no layout when a pane closes, so the pane left
        // behind kept the closed one's half of the grid until something else
        // read the layout: a click (checklist O1). Read it now; the view
        // re-sends its viewport when the arrangement changes.
        await store.client(for: worktree)?.refreshLayout(worktree)
        // Nothing to select here. Where the selection goes when a terminal
        // disappears is `healSelection`'s one rule, run from
        // `.onChange(of: store.fleet)` once the removal reaches the merged
        // fleet. A neighbour picked here would be chosen before the removal
        // reached `store.fleet`, find the closed terminal still listed, and
        // leave `healSelection` nothing to heal: ⌘W in one worktree once landed
        // you in another, often on another runner.
    }

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
            // As ⌘W, which asks first when the pane is mid-turn.
            requestClose(term, in: worktree)
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
