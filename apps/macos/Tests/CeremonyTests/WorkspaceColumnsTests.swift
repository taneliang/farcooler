import AppKit
import CoreGraphics
import SwiftUI
import Testing

@testable import Far_Cooler

/// The workspace's columns at the widths 2A measured (spec §4.3). The detail
/// is the window less a 248 pt sidebar: 1222 pt at a full-screen 1470, 1192 at
/// 1440, and 1032 in a 1280 pt window.
struct WorkspaceColumnsTests {
    private typealias Columns = WorkspaceColumns

    /// Each minimum is the narrowest width holding its columns: one point
    /// less loses one. That's 48 columns for the conversation and 58 for the
    /// task at the default font.
    @Test("The minimums hold their measured columns and no more")
    func theMinimumsHoldTheirMeasuredColumns() {
        func columns(_ width: CGFloat) -> Int { Int((width - Columns.chrome) / Columns.defaultCell) }
        #expect(columns(Columns.conversationMinimum()) == 48)
        #expect(columns(Columns.conversationMinimum() - 1) == 47)
        #expect(columns(Columns.taskMinimum()) == 58)
        #expect(columns(Columns.taskMinimum() - 1) == 57)
        #expect(Columns.taskMinimum() == 489)
        // At 13 pt they're 426 and 507, and three no longer fit at 1440.
        let thirteen: CGFloat = 8.0361328125
        #expect(Columns.conversationMinimum(cell: thirteen) == 426)
        #expect(Columns.taskMinimum(cell: thirteen) == 507)
        #expect(Columns.layout(width: 1192, taskOpen: true, cell: thirteen) == .railed)
    }

    @Test("All three fit at the measured full-screen 13-inch width")
    func allThreeFitAtTheMeasuredFullScreenWidth() {
        #expect(Columns.layout(width: 1222, taskOpen: true) == .all)
        #expect(Columns.layout(width: 1192, taskOpen: true) == .all)
        // Hiding the sidebar in a 1280 pt window.
        #expect(Columns.layout(width: 1280, taskOpen: true) == .all)
    }

    /// And a 1280 pt window with its sidebar has the conversation and the
    /// board until a task opens. Focus Column puts the task over both.
    @Test("Below it, opening a task collapses the conversation to its rail")
    func belowItOpeningATaskCollapsesTheConversationToItsRail() {
        #expect(Columns.layout(width: 1032, taskOpen: false) == .two)
        #expect(Columns.layout(width: 1032, taskOpen: true) == .railed)
        let three = Columns.conversationMinimum() + Columns.boardMinimum + Columns.taskMinimum() + 2
        #expect(Columns.layout(width: three, taskOpen: true) == .all)
        #expect(Columns.layout(width: three - 1, taskOpen: true) == .railed)
        let railed = Columns.rail + Columns.boardMinimum + Columns.taskMinimum() + 2
        #expect(Columns.layout(width: railed, taskOpen: true) == .railed)
        #expect(Columns.layout(width: railed - 1, taskOpen: true) == .taskAlone)
        #expect(Columns.layout(width: 1222, taskOpen: true, focused: true) == .taskAlone)
    }

    /// Down to the window's 600 pt minimum, where the detail is 352 pt.
    @Test("Below the two-column minimum it's one column with Orchestrator | Board")
    func belowTheTwoColumnMinimumItsOneColumn() {
        let two = Columns.conversationMinimum() + Columns.boardMinimum + 1
        #expect(Columns.layout(width: two, taskOpen: false) == .two)
        #expect(Columns.layout(width: two - 1, taskOpen: false) == .one)
        #expect(Columns.layout(width: 352, taskOpen: false).switcher)
        // A workspace with no conversation, on a runner without workstreams:
        // the board, then the task beside it, or alone.
        let implicit = Columns.layout(width: 352, taskOpen: false, hasConversation: false)
        #expect(implicit.board && !implicit.switcher && implicit.conversation == .none)
        #expect(Columns.layout(width: 1000, taskOpen: true, hasConversation: false).board)
        #expect(Columns.layout(width: 700, taskOpen: true, hasConversation: false) == .taskAlone)
    }

    /// The drawn view, hosted in a detail 1032 pt wide inside a 1600 pt
    /// window: it lays out by its own width, so a task collapses the
    /// conversation to its rail. A view that read the window would draw all
    /// three and clip.
    @MainActor
    @Test("The workspace lays out by its own width, not the window's")
    func theWorkspaceLaysOutByItsOwnWidth() async {
        final class Seen { var arrangement: WorkspaceColumns.Arrangement? }
        let seen = Seen()
        struct Hosted: View {
            let seen: Seen
            let taskOpen: Bool
            var body: some View {
                WorkspaceView(
                    taskOpen: taskOpen, hasConversation: true, cell: WorkspaceColumns.defaultCell, focused: false,
                    pick: .constant(.orchestrator),
                    conversation: { Color.clear }, rail: { Color.clear }, board: { Color.clear },
                    third: { Color.clear })
                .frame(width: 1032, height: 400)
                .frame(width: 1600, height: 400, alignment: .leading)
                .onPreferenceChange(WorkspaceArrangementPreference.self) { value in
                    MainActor.assumeIsolated { seen.arrangement = value }
                }
            }
        }
        for (taskOpen, expected) in [(true, WorkspaceColumns.Arrangement.railed), (false, .two)] {
            seen.arrangement = nil
            let host = NSHostingView(rootView: Hosted(seen: seen, taskOpen: taskOpen))
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 1600, height: 400), styleMask: [.borderless],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
            for _ in 0..<5 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
            window.close()
            #expect(seen.arrangement == expected)
        }
    }
}
