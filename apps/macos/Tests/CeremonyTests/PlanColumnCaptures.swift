import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the plan column beside the orchestrator
/// (ov-867, C10f) at 1,100, 1,440 and 1,920 points, light and dark: the
/// production `WorkspaceView` with `PlanHome` over `PlanColumnScenario` and a
/// conversation in the chat column, in a real titled window off every screen,
/// taken with `screencapture -l`. Below 1,180 the canvas folds, so 1,100 shows
/// the chat with the plan's strip. Sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct PlanColumnCaptures {
    typealias T = NativeAgentTests

    @Test(arguments: [1100.0, 1440.0, 1920.0])
    func column(_ width: CGFloat) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let board = try await PlanColumnViewTests.store()
        let height: CGFloat = 1900
        let expecting = "the C10f plan read"
        for (variant, appearance) in RealWindowCaptures.variants.prefix(2) {
            let model = T.model(try T.terminal())
            model.store.apply(try await model.store.ledger.page(T.page(NativeAgentSubagentTests.rows())))
            let reached = { @MainActor in PlanColumnScenario.shown(board.plan.plan) }
            let view = WorkspaceView(
                opened: nil as String?, hasConversation: true, hasBoard: false, cell: WorkspaceColumns.defaultCell,
                focused: false, navigatorWidth: .constant(320),
                conversation: { NativeAgentView(model: model, isFocused: true, showTerminal: {}).paneCard().paneCanvas() },
                navigator: { Color.clear }, breadcrumb: { _ in Color.clear }, detail: { _, _ in Color.clear }, split: true,
                home: { AnyView(PlanHome(board: board, needsYou: PlanNeedsYou(), onOpen: { _ in })) },
                boardExists: false,
                cards: { canvas in
                    canvas ? AnyView(EmptyView()) : AnyView(
                        WorkspaceStripRow {
                            PlanStripRow(plan: board.plan, needsYou: 2, statuses: board.board.statuses) {}
                        })
                })
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(rootView: view.frame(width: width, height: height).background { WindowPlane() })
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            try await Capture.keep(
                window, to: URL(fileURLWithPath: out).appendingPathComponent("c10f-plan-\(Int(width))-\(variant).png"), expecting: expecting,
                until: reached)
            window.close()
        }
    }

    @Test(arguments: [1440.0])
    func ticket(_ width: CGFloat) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let board = try await PlanColumnViewTests.store()
        let height: CGFloat = 1000
        let ticket = TaskRow(
            id: "ov-880", key: "ov-880", title: "The Android app no longer crashes on a cold start after the Play update",
            status: .inProgress, statusSince: Date(),
            intent: "A cold start after the Play update crashes before the first frame. Find why, and make the app reach its home screen.",
            acceptance: [
                TaskAcceptanceLine(id: "a1", text: "A cold start after an update reaches the home screen", met: true),
                TaskAcceptanceLine(id: "a2", text: "A test starts the app from a cleared cache", met: false),
            ])
        let expecting = "the C10f plan read"
        for (variant, appearance) in RealWindowCaptures.variants.prefix(2) {
            let model = T.model(try T.terminal())
            model.store.apply(try await model.store.ledger.page(T.page(NativeAgentSubagentTests.rows())))
            let reached = { @MainActor in PlanColumnScenario.shown(board.plan.plan) }
            let view = WorkspaceView(
                opened: "ov-881", hasConversation: true, hasBoard: false, cell: WorkspaceColumns.defaultCell,
                focused: false, navigatorWidth: .constant(320),
                conversation: { NativeAgentView(model: model, isFocused: true, showTerminal: {}).paneCard().paneCanvas() },
                navigator: { Color.clear }, breadcrumb: { _ in
                    DrillBreadcrumb(
                        crumbs: PlanStack.page(
                            base: [
                                .init(title: "Far Cooler", target: PlanStackTests.plan),
                                .init(title: "Android", target: PlanStackTests.ticket),
                                .init(title: "The Android app no longer crashes on a cold start after the Play update", target: nil),
                            ],
                            place: PlanStackTests.task("ov-880"),
                            origin: PlanStack.Origin(task: "ov-880", agent: "Builder 1", target: PlanStackTests.ticket),
                            besidePlan: true
                        ).crumbs, onGo: { _ in }, onClose: nil)
                },
                detail: { _, _ in
                    PlanTicketPage(
                        row: ticket,
                        work: PlanTicketWork(
                            agent: "Builder 1", doing: "Running the Android tests", branch: "builder-1/android-cold-start",
                            openChat: {}, openTerminal: {}, openChanges: {}),
                        line: { EmptyView() },
                        card: {
                            TaskCard(row: ticket, detail: .empty, question: nil, canAnswer: true, onAnswer: { _ in true })
                        }
                    ).contentCard()
                }, split: true,
                home: { AnyView(PlanHome(board: board, needsYou: PlanNeedsYou(), onOpen: { _ in })) },
                boardExists: false,
                cards: { canvas in
                    canvas ? AnyView(EmptyView()) : AnyView(
                        WorkspaceStripRow {
                            PlanStripRow(plan: board.plan, needsYou: 2, statuses: board.board.statuses) {}
                        })
                })
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(rootView: view.frame(width: width, height: height).background { WindowPlane() })
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            try await Capture.keep(
                window, to: URL(fileURLWithPath: out).appendingPathComponent("ticket-plan-\(Int(width))-\(variant).png"), expecting: expecting,
                until: reached)
            window.close()
        }
    }
}
