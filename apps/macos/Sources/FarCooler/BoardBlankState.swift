import SwiftUI

/// What the Tasks section says on a board with no task at all (ov-205).
///
/// Unread's "You're all caught up." is about a list that had something in it,
/// so a blank board says what tasks are for instead: one short line of
/// purpose, then rows of a symbol and a few words, as every other empty state.
struct BoardBlankState: View {
    static let copy = EmptyStateCopy(
        lede: "The orchestrator turns your requests into tasks.",
        rows: [
            .init(symbol: "person.crop.circle.badge.checkmark", text: "An agent works on each one"),
            .init(symbol: "questionmark.bubble", text: "It asks you when it’s stuck"),
            .init(symbol: "eye", text: "Finished work waits for your review"),
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
