import AgentKit
import SwiftUI

// Moved out of TaskBoard.swift, which is at its size ceiling (ov-321); nothing
// in it changed.

/// Rows this build has no column for.
///
/// A runner ahead of this app can name a status it has never heard of. Showing
/// it under a heading that says so is the only honest answer: dropping the row
/// makes work vanish from a board whose whole claim is that it shows the work.
struct UnreadableColumnView: View {
    let rows: [UnreadableTaskRow]

    /// A section like the statuses above it: its heading at column B, and
    /// its rows as cards with their edges at A and their text at B.
    var body: some View {
        VStack(alignment: .leading, spacing: NavigatorRhythm.group) {
            VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                Text("Not On This Version").font(WorkspaceStyle.sectionTitle)
                    .gridMark("unreadable", .text)
                Text("This runner uses states this Far Cooler doesn’t have yet.")
                    .font(.system(size: WorkspaceStyle.PaneText.secondary))
                    .foregroundStyle(SidebarInk.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.leading, NavigatorGrid.textInset)
            ForEach(rows) { row in
                VStack(alignment: .leading, spacing: NavigatorRhythm.lineGap) {
                    Text(row.key)
                        .font(
                            .system(
                                size: WorkspaceStyle.PaneText.secondary, design: .monospaced)
                        )
                        .foregroundStyle(SidebarInk.secondary)
                    Text(row.title).font(.system(size: WorkspaceStyle.PaneText.body))
                    Text(row.status)
                        .font(.system(size: WorkspaceStyle.PaneText.minimum, design: .monospaced))
                        .foregroundStyle(SidebarInk.secondary)
                }
                .padding(.horizontal, NavigatorGrid.textInset)
                .padding(.vertical, NavigatorRhythm.card)
                .frame(maxWidth: .infinity, alignment: .leading)
                .surface(.inset, in: .card)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
