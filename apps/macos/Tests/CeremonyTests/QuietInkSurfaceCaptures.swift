import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the Plan view and the board's task card on
/// their paper, light and dark, for judging that the hierarchy still reads once
/// quiet text is `Ink.quiet` (ov-687). The production views over the plan
/// fixture the stage captures use, in a real titled window off every screen.
/// For the before, run it on the commit before the change.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct QuietInkSurfaceCaptures {
    static let appearances: [(String, NSAppearance.Name)] = [("light", .aqua), ("dark", .darkAqua)]

    /// `view` on the paper a content card is drawn on, in each appearance.
    static func shoot<V: View>(
        _ name: String, width: CGFloat, height: CGFloat, expecting: String, until reached: @MainActor () -> Bool, _ view: V
    ) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for (variant, appearance) in appearances {
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(
                rootView: view.frame(width: width, height: height, alignment: .topLeading).background(WorkspaceStyle.paper))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            try await Capture.wait(expecting: expecting, until: reached)
            await NativeAgentTests.settle(window, 900)
            try await Capture.keep(
                to: URL(fileURLWithPath: out).appendingPathComponent("\(name)-\(variant).png"),
                expecting: expecting, until: reached
            ) { window.contentView?.lookBitmap(scale: 2)?.representation(using: .png, properties: [:]) }
            window.close()
        }
    }

    static func store() async throws -> TaskBoardStore {
        let store = try await PlanStageViewTests.store()
        await store.plan.reload()
        return store
    }

    @Test func planHome() async throws {
        let store = try await Self.store()
        try await Self.shoot(
            "quiet-plan-home", width: 760, height: 1_350, expecting: "the plan read",
            until: { store.plan.planned && !store.plan.plan.lanes.isEmpty },
            PlanHome(board: store, needsYou: PlanNeedsYou(), onOpen: { _ in }))
    }

    @Test func lanePage() async throws {
        let store = try await Self.store()
        let plan = store.plan.plan
        try await Self.shoot(
            "quiet-plan-lane", width: 640, height: 700, expecting: "the plan read",
            until: { store.plan.plan.lanes.count >= 5 },
            PlanLanePage(
                lane: plan.lanes[4], plan: plan, record: nil,
                context: PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })))
    }

    @Test func themePage() async throws {
        let plan = try PlanThemePageTests.model()
        let theme = try #require(plan.shownThemes.first)
        try await Self.shoot(
            "quiet-plan-theme", width: 700, height: 900, expecting: "the theme page", until: { true },
            PlanThemePage(
                theme: theme, plan: plan, record: nil,
                context: PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })))
    }

    static let record = """
        {"task": {}, "notes": [
          {"id": "n1", "kind": "decision", "actor": "manager", "at": 1760000000000,
           "body": "Use the quiet ink for every secondary line on the paper.", "extra": {"rejected": ["Darken .secondary", "A new gray"]}},
          {"id": "n2", "kind": "finding", "actor": "agent:7b1c", "at": 1759990000000, "body": "The system secondary measures 3.8:1 on the paper."},
          {"id": "n3", "kind": "progress", "actor": "agent:7b1c", "at": 1759980000000, "body": "Plan view done; the board is next."},
          {"id": "n4", "kind": "status_change", "actor": "manager", "at": 1759970000000, "body": "todo to in_progress"},
          {"id": "n5", "kind": "created", "actor": "user", "at": 1759960000000, "body": "Mac: secondary text reads in light appearance"}
        ]}
        """

    static let spend = """
        {"task":"t","price_table":"2026-09-25",
         "totals":{"turns":12,"turns_partial":1,"active_ms":10200000,"input_tokens":251000,"output_tokens":27400,
                   "cache_read_tokens":1084000,"cache_write_tokens":62000,"cost_reported_micros":2870000,
                   "unpriced_tokens":283000},
         "by_harness_model":[
           {"harness":"claude","model":"claude-opus-5","totals":{"turns":9,"active_ms":7800000,
             "input_tokens":41000,"output_tokens":18400,"cache_read_tokens":1020000,"cache_write_tokens":62000,
             "cost_reported_micros":2870000}}]}
        """

    @Test func taskCard() async throws {
        let detail = try TaskDetailModel.decode(Data(Self.record.utf8))
        let usage = try TaskUsage.decode(Data(Self.spend.utf8))
        let row = TaskRow(
            id: "t1", key: "ov-687", title: "Mac: secondary text reads in light appearance", status: .inProgress,
            statusSince: Date(timeIntervalSinceNow: -3 * 86_400),
            intent: "In Light Appearance, .secondary text on the Mac measures under 4:1 on the paper.",
            labels: ["found"],
            acceptance: [
                TaskAcceptanceLine(id: "1", text: "Each surface on our own ground reads at 4.5:1", met: true),
                TaskAcceptanceLine(id: "2", text: "System surfaces keep .secondary, with reasons", met: false),
            ],
            constraints: ["No cargo fmt"], createdAt: Date(timeIntervalSinceNow: -5 * 86_400), updatedAt: Date(timeIntervalSinceNow: -3_600))
        try await Self.shoot(
            "quiet-task-card", width: 640, height: 1_000, expecting: "the card", until: { true },
            TaskCard(row: row, detail: detail, question: nil, canAnswer: true, usage: .loaded(usage), onAnswer: { _ in true })
                .padding(TaskTypography.inset))
    }
}
