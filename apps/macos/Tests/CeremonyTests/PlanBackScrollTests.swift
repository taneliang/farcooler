import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Back from a ticket opened from the plan returns to the plan where it was
/// (ov-873): the plan stays mounted beneath, so its scroll offset is the same
/// one. The real `WorkspaceView` with `PlanHome`, scrolled, then a ticket
/// opened, then closed again.
@MainActor
@Suite(.serialized)
struct PlanBackScrollTests {
    typealias T = NativeAgentTests

    static func scrollViews(in view: NSView) -> [NSScrollView] {
        (view as? NSScrollView).map { [$0] } ?? [] + view.subviews.flatMap { scrollViews(in: $0) }
    }

    /// The window's layout and scroll offsets have held still: what a fixed settle stood in for.
    static func still(_ window: NSWindow) async {
        await Settle.still(
            window.contentView,
            reading: {
                window.displayIfNeeded()
                return Settle.frames(under: window.contentView) + Settle.scrollOffsets(under: window.contentView)
            })
    }

    @Test("Scroll the plan, open a ticket, go Back: the plan is at the same offset")
    func backRestoresThePlansScroll() async throws {
        let board = try await PlanColumnViewTests.store()
        let model = T.model(try T.terminal())
        let width: CGFloat = 1440, height: CGFloat = 700
        func root(_ opened: String?) -> some View {
            WorkspaceView(
                opened: opened, hasConversation: true, hasBoard: false, cell: WorkspaceColumns.defaultCell,
                focused: false, navigatorWidth: .constant(320),
                conversation: { NativeAgentView(model: model, isFocused: false, showTerminal: {}).paneCard().paneCanvas() },
                navigator: { Color.clear }, breadcrumb: { _ in Color.clear },
                detail: { _, _ in PlanTicketPageTests.page(PlanTicketPageTests.work).contentCard() }, split: true,
                home: { AnyView(PlanHome(board: board, needsYou: PlanNeedsYou(), onOpen: { _ in })) },
                boardExists: false,
                cards: { canvas in
                    canvas ? AnyView(EmptyView()) : AnyView(
                        WorkspaceStripRow { PlanStripRow(plan: board.plan, needsYou: 2, statuses: board.board.statuses) {} })
                }
            ).frame(width: width, height: height)
        }
        let window = NSWindow(
            contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let host = NSHostingView(rootView: AnyView(root(nil)))
        window.contentView = host
        TestWindow.show(window)
        func tallest() -> NSScrollView? {
            Self.scrollViews(in: host).filter { ($0.documentView?.frame.height ?? 0) > $0.contentView.bounds.height + 200 }
                .max { ($0.documentView?.frame.height ?? 0) < ($1.documentView?.frame.height ?? 0) }
        }
        try await Capture.wait(expecting: "the plan is laid out and tall enough to scroll", until: { tallest() != nil })
        await Self.still(window)
        let plan = try #require(tallest(), "the plan's scroll view")
        let scrolled = min(300, (plan.documentView?.frame.height ?? 0) - plan.contentView.bounds.height)
        plan.contentView.scroll(to: NSPoint(x: 0, y: scrolled))
        plan.reflectScrolledClipView(plan.contentView)
        await Self.still(window)
        let before = plan.contentView.bounds.origin.y
        #expect(before > 100, "the plan scrolled: \(before)")

        host.rootView = AnyView(root("ov-880"))
        await Self.still(window)
        host.rootView = AnyView(root(nil))
        await Self.still(window)

        #expect(plan.window === window, "the same plan view is still mounted")
        #expect(abs(plan.contentView.bounds.origin.y - before) < 1, "back at \(plan.contentView.bounds.origin.y), was \(before)")
        window.close()
    }
}
