import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A pull request's stage on the Mac's plan (ov-316): the words on a ticket row,
/// the card page's stage line and the ticket page, over the runner's own
/// bytes (`test/fixtures/plan-stages.json`, written by
/// `plan_stages_json_is_the_shape_the_mac_reads`). Ticket `ov-N` is card
/// `ov-N`, whose task id ends in the hex of 0x1000 + N.
@MainActor
@Suite(.serialized)
struct PlanStageViewTests {
    static func bytes() throws -> Data {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try Data(contentsOf: root.appendingPathComponent("test/fixtures/plan-stages.json"))
    }

    static func store() async throws -> TaskBoardStore {
        let calls = PlanViewTests.Calls()
        calls.plan = try bytes()
        return try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
    }

    static func task(_ n: Int) -> String { "00000000-0000-0000-0000-" + String(format: "%012x", 0x1000 + n) }

    /// What a view drew, by identifier, and where, at `width`.
    static func drawn(_ view: some View, width: CGFloat) async -> [String: CGRect] {
        let seen = NavigatorFilterTests.Seen()
        let host = NSHostingView(
            rootView: view
                .frame(width: width, alignment: .topLeading)
                .fixedSize(horizontal: false, vertical: true)
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { geo in
                        let _ = seen.views = Dictionary(probed.map { ($0.id, geo[$0.bounds]) }, uniquingKeysWith: { a, _ in a })
                        Color.clear
                    }
                })
        host.frame = CGRect(x: 0, y: 0, width: width, height: 400)
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return seen.views
    }

    @Test("A card's line shows its pull request's stage, and shows none for a building card or a runner that said nothing")
    func taskLineShowsTheStage() async throws {
        let store = try await Self.store()
        await store.plan.reload()
        func line(_ n: Int, width: CGFloat = 600) async -> [String: CGRect] {
            await Self.drawn(PlanTaskLineView(plan: store.plan, task: Self.task(n)) { _ in }, width: width)
        }
        #expect(await line(4)["plan-task-stage"] != nil, "waiting on a reviewer")
        #expect(await line(14)["plan-task-stage"] != nil, "PR state unknown is said, never left out as no PR")
        #expect(await line(1)["plan-task-stage"] == nil, "Building is the ticket's own word")
        #expect(await line(1)["plan-task-ticket-stage"] != nil, "and the ticket's stage is said")
        let silent = await line(256)
        #expect(silent["plan-task-ticket-stage"] != nil && silent["plan-task-stage"] == nil, "a ticket the runner said nothing of has its stage only")
        #expect(await line(300)["plan-task-line"] == nil, "a card on no ticket and in no theme says nothing")
    }

    @Test("A long reviewer login never squeezes the line: it stacks under the stage and stays inside the column")
    func longLoginStacks() async throws {
        let store = try await Self.store()
        await store.plan.reload()
        let view = { PlanTaskLineView(plan: store.plan, task: Self.task(5)) { _ in } }
        let wide = await Self.drawn(view(), width: 900)
        let narrow = await Self.drawn(view(), width: 260)
        let wideLine = try #require(wide["plan-task-line"])
        let narrowLine = try #require(narrow["plan-task-line"])
        let stage = try #require(narrow["plan-task-stage"])
        #expect(narrowLine.height > wideLine.height * 1.5, "stacked at 260, one line at 900: \(narrowLine) \(wideLine)")
        #expect(stage.maxX <= 260 + 1, "inside the column: \(stage)")
        // Stacked, it starts at the column's edge; squeezed beside the stage's
        // words it would start after them (the mutation that drew one HStack).
        #expect(stage.minX <= narrowLine.minX + 1, "stacked, not squeezed: \(stage)")
        let wideStage = try #require(wide["plan-task-stage"])
        #expect(stage.height > 0 && wideStage.height <= wideLine.height + 1)
        let shortStage = try #require(await Self.drawn(PlanTaskLineView(plan: store.plan, task: Self.task(4)) { _ in }, width: 260)["plan-task-stage"])
        #expect(stage.height <= shortStage.height + 1, "elided to one line, not wrapped: \(stage) \(shortStage)")
    }

    @Test("A ticket row draws only a stage that needs a person in color, on a line of its own")
    func ticketRowColorsOnlyWhatNeedsAPerson() async throws {
        let store = try await Self.store()
        await store.plan.reload()
        let lines = store.plan.plan.lines
        func row(_ n: Int) async -> [String: CGRect] {
            let line = lines[n - 1]
            return await Self.drawn(
                PlanLineRow(
                    line: line, theme: nil, rank: nil, now: store.plan.plan.nowMs, waitsOnOwner: false, selected: false,
                    keyed: false, action: {}), width: 420)
        }
        // Amber exactly when the stage says it needs a person, read from the
        // stage and not from a list of tickets.
        for n in 1...15 {
            let line = lines[n - 1]
            let drawn = await row(n)
            let amber = drawn["plan-ticket-\(line.key)-stage-attention"] != nil
            let gray = drawn["plan-ticket-\(line.key)-stage"] != nil
            #expect(amber == (line.shownStage?.needsAttention == true), "\(line.key) amber")
            #expect(gray == (line.shownStage != nil && line.shownStage?.needsAttention == false), "\(line.key) gray")
        }
        #expect(lines.filter { $0.shownStage?.needsAttention == true }.count == 3, "the fixture holds three that need a person")
        // The state line keeps the state; the stage has a line of its own.
        func second(_ n: Int) -> String {
            PlanLineRow.second(lines[n - 1], agent: nil, landedLanding: nil, now: store.plan.plan.nowMs, aged: false,
                state: lines[n - 1].shownStage == nil ? nil : PlanWords.status(lines[n - 1], stage: false))
        }
        #expect(second(6).hasPrefix("In Review"))
        #expect(second(8).hasPrefix("Approved \u{00B7} "))
        #expect(second(4).hasPrefix("In Review"))
        #expect(second(14).hasPrefix("In Review"))
        #expect(second(15).hasPrefix("In Review"))
    }

    @Test("A long reviewer login stays on one line in a row, elided inside the column")
    func longLoginElidesInARow() async throws {
        let store = try await Self.store()
        await store.plan.reload()
        let lines = store.plan.plan.lines
        func stage(_ n: Int, _ width: CGFloat) async throws -> CGRect {
            let drawn = await Self.drawn(
                PlanLineRow(
                    line: lines[n - 1], theme: nil, rank: nil, now: store.plan.plan.nowMs, waitsOnOwner: false, selected: false,
                    keyed: false, action: {}), width: width)
            return try #require(drawn["plan-ticket-ov-\(n)-stage"])
        }
        let short = try await stage(4, 260)
        let long = try await stage(5, 260)
        #expect(long.height <= short.height + 1, "one line like the short one: \(long) \(short)")
        #expect(long.maxX <= 260, "inside the column: \(long)")
    }

    @Test("The ticket page says the stage under its title, with the pull request to open, and says nothing for a silent ticket")
    func ticketPageStage() async throws {
        let store = try await Self.store()
        await store.plan.reload()
        let plan = store.plan.plan
        func page(_ n: Int) async -> [String: CGRect] {
            await Self.drawn(
                PlanTicketRecordPage(
                    line: plan.lines[n - 1], plan: plan, record: nil,
                    context: PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })),
                width: 560)
        }
        #expect(await page(4)["plan-ticket-stage"] != nil)
        #expect(await page(6)["plan-ticket-stage"] != nil)
        #expect(PlanWords.status(plan.lines[5], stage: false) == "In Review", "the title keeps the state; the stage has its line")
        // A long login on the ticket page: one line, then the pull request under it.
        let narrow = await Self.drawn(
            PlanTicketRecordPage(line: plan.lines[3], plan: plan, record: nil, context: PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })),
            width: 300)
        let stage = try #require(narrow["plan-ticket-stage"])
        let wide = try #require(await page(4)["plan-ticket-stage"])
        #expect(stage.height > wide.height * 1.5, "PR 31 goes under the stage when stacked: \(stage) \(wide)")
        #expect(await page(1)["plan-ticket-stage"] == nil)
        #expect(await page(15)["plan-ticket-stage"] == nil)
    }
}
