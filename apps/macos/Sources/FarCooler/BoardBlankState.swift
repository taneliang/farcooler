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

    var body: some View {
        EmptyStateRows(copy: Self.copy)
            .font(.system(size: WorkspaceStyle.PaneText.body))
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, minHeight: NavigatorRhythm.placeholder)
            .padding(.horizontal, NavigatorGrid.textInset)
            .identified("board-empty")
    }
}
