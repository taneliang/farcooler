import AgentKit
import SwiftUI

/// What the Tasks section says on a board with no task at all (ov-205).
///
/// Unread's "You're all caught up." is about a list that had something in it,
/// so a blank board says what tasks are for instead: one short line of
/// purpose, then rows of a symbol and a few words, as every other empty state.
struct BoardBlankState: View {
    static let copy = EmptyStateCopy(
        lede: "Tasks show up here as the orchestrator makes them.",
        rows: [
            .init(symbol: "bubble.left", text: "Tell it what you want built"),
            .init(symbol: "eye", text: "Finished work waits here for your review"),
        ])

    /// With no orchestrator to make them: where tasks come from first, the
    /// phones' words (`PhoneEmptyStates.boardNoOrchestrator`).
    static let copyWithoutOrchestrator = EmptyStateCopy(
        lede: PhoneEmptyStates.boardNoOrchestrator.lede,
        rows: PhoneEmptyStates.boardNoOrchestrator.rows.map { .init(symbol: $0.symbol, text: $0.text) })

    @Environment(\.orchestratorInvite) private var invite

    /// Whether to offer Start Orchestrator: none is seated, and this runner
    /// lets this window start one.
    private var offersStart: Bool { invite.map { !$0.seated && $0.canAct } ?? false }

    var body: some View {
        VStack(spacing: Spacing.inset) {
            EmptyStateRows(copy: invite.map { $0.seated } ?? true ? Self.copy : Self.copyWithoutOrchestrator)
                .font(.system(size: WorkspaceStyle.PaneText.body))
                .foregroundStyle(SidebarInk.secondary)
            if offersStart, let invite { StartOrchestratorMenu(invite: invite, captionInk: SidebarInk.secondary) }
        }
        .frame(maxWidth: .infinity, minHeight: NavigatorRhythm.placeholder)
        .padding(.horizontal, NavigatorGrid.textInset)
        .identified("board-empty")
    }
}
