import AgentKit
import SwiftUI

/// What the owner's three actions on a ruling do in this window (ov-333),
/// handed down by the environment so the home, the peek, a theme's page and the
/// tests all draw the same rows. Keep is the owner's own mark, written to the
/// runner; Reverse and Discuss reach the workspace's orchestrator.
struct PlanRulingActions {
    /// Whether this runner takes the owner's marks (`board_ruling_actions`).
    /// Without it the rows offer Copy Reference only.
    var canKeep = false
    /// Whether the workspace has an orchestrator running to ask.
    var canAsk = false
    /// Show the row actions without the pointer on the row: a capture of the
    /// real window can't hover (`FARCOOLER_CAPTURE_RULING_ACTIONS`, set by the
    /// capture script's caller).
    var alwaysShown = false
    var keep: (PlanRuling) -> Void = { _ in }
    var keepAll: () -> Void = {}
    var reverse: (PlanRuling) -> Void = { _ in }
    var discuss: (PlanRuling) -> Void = { _ in }

    // Closures that only the main actor calls, so safe to share: the same
    // reasoning as the other environment values that carry callbacks.
    nonisolated(unsafe) static let none = PlanRulingActions()
}

private struct PlanRulingActionsKey: EnvironmentKey {
    nonisolated(unsafe) static let defaultValue = PlanRulingActions.none
}

extension EnvironmentValues {
    var planRulingActions: PlanRulingActions {
        get { self[PlanRulingActionsKey.self] }
        set { self[PlanRulingActionsKey.self] = newValue }
    }
}

extension ContentView {
    /// The owner's actions on `workspace`'s rulings: Keep and Keep All to the
    /// runner, Reverse to the orchestrator's chat (sent), Discuss into its
    /// composer (unsent). Reverse and Discuss are off while it has no
    /// orchestrator running, as Ask the Orchestrator is, and never start one.
    func planRulingActions(host: String, workspace: WorkspaceSummary, plan: PlanStore) -> PlanRulingActions {
        let live = WorkspaceScreen.workspace(
            workspace.id, host: host, in: store.fleet,
            repositories: store.clients[host]?.repositories.map(\.id) ?? []) ?? workspace
        let seat = WorkspaceScreen.orchestrator(of: live, host: host, in: store.fleet)
        let client = seat.flatMap { store.client(for: $0.worktree) }
        return PlanRulingActions(
            canKeep: plan.canMarkRulings,
            canAsk: seat != nil,
            alwaysShown: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_RULING_ACTIONS"] != nil,
            keep: { ruling in Task { await plan.keep(ruling) } },
            keepAll: { Task { await plan.keepAll() } },
            reverse: { ruling in
                guard let seat else { return }
                selection = .workspace(host: host, workspace: workspace.id, focus: nil)
                Task { @MainActor in
                    let outcome = await RulingOrchestrator.reverse(ruling, seat: seat, client: client)
                    if outcome != .sent {
                        focus(PaneRef(host: host, worktree: seat.worktree.id, terminal: seat.terminal.id))
                    }
                    if let notice = RulingActions.notice(for: outcome, ruling: ruling) { errorBanner = notice }
                }
            },
            discuss: { ruling in
                guard let seat else { return }
                selection = .workspace(host: host, workspace: workspace.id, focus: nil)
                Task { @MainActor in
                    let delivery = await RulingOrchestrator.discuss(ruling, seat: seat, client: client)
                    if delivery != .composer {
                        focus(PaneRef(host: host, worktree: seat.worktree.id, terminal: seat.terminal.id))
                    }
                    if delivery == .copied {
                        errorBanner = "Copied the start of a message about \(ruling.short). Paste it into the orchestrator."
                    }
                }
            })
    }
}

/// Where Reverse and Discuss go: the workspace's orchestrator, as
/// `AskOrchestrator` reaches it. Here, apart from the window, so a test can
/// hand it a pane and a client and read what was sent.
@MainActor
enum RulingOrchestrator {
    /// Reverse's request, sent to a chat orchestrator as a typed message is
    /// (`terminal agent-prompt`) and typed with no Enter into a terminal one.
    static func reverse(_ ruling: PlanRuling, seat: BoardPane, client: DaemonClient?) async -> RulingActions.Reversal {
        await RulingActions.reverse(
            ruling, isAgentPane: seat.terminal.isAgentPane,
            send: { text in await client?.agentPrompt(terminal: seat.terminal.short, text: text) == nil },
            paste: { text in await paste(text, seat: seat, client: client) },
            copy: client?.copyToClipboard ?? AskOrchestrator.copyToPasteboard)
    }

    /// Discuss's draft, left in a chat orchestrator's composer, unsent.
    static func discuss(
        _ ruling: PlanRuling, seat: BoardPane, client: DaemonClient?, handoff: ComposerHandoff = .shared
    ) async -> AskAboutTask.Delivery {
        await RulingActions.discuss(
            ruling, isAgentPane: seat.terminal.isAgentPane,
            offer: { handoff.offer($0, to: seat.terminal.short) },
            paste: { text in await paste(text, seat: seat, client: client) },
            copy: client?.copyToClipboard ?? AskOrchestrator.copyToPasteboard)
    }

    private static func paste(_ text: String, seat: BoardPane, client: DaemonClient?) async -> AskAboutTask.DraftResult {
        await client?.draftPrompt(terminal: seat.terminal.short, text: text) == true ? .pasted : .declined
    }
}
