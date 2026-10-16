import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A pull request's stage on the Mac's plan (ov-316): the words on a lane row,
/// the card page's "In lane" line and the lane page, over the runner's own
/// bytes (`test/fixtures/plan-stages.json`, written by
/// `plan_stages_json_is_the_shape_the_mac_reads`). Lane `lane-N` works card
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
        #expect(await line(1)["plan-task-stage"] == nil, "Building is the lane's own word")
        #expect(await line(15)["plan-task-line"] == nil, "the silent lane's card is in no lane here")
    }

    @Test("A long reviewer login never squeezes the line: it stacks under the lane and stays inside the column")
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
        // Stacked, it starts at the column's edge; squeezed beside the lane's
        // button it would start after it (the mutation that drew one HStack).
        #expect(stage.minX <= narrowLine.minX + 1, "stacked, not squeezed: \(stage)")
        let wideStage = try #require(wide["plan-task-stage"])
        #expect(stage.height > 0 && wideStage.height <= wideLine.height + 1)
        let shortStage = try #require(await Self.drawn(PlanTaskLineView(plan: store.plan, task: Self.task(4)) { _ in }, width: 260)["plan-task-stage"])
        #expect(stage.height <= shortStage.height + 1, "elided to one line, not wrapped: \(stage) \(shortStage)")
    }

    @Test("A lane row draws only a stage that needs a person in color, on a line of its own")
    func laneRowColorsOnlyWhatNeedsAPerson() async throws {
        let store = try await Self.store()
        await store.plan.reload()
        let lanes = store.plan.plan.lanes
        func row(_ n: Int) async -> [String: CGRect] {
            let lane = lanes[n - 1]
            return await Self.drawn(
                PlanLaneRow(
                    lane: lane, theme: nil, rank: nil, now: store.plan.plan.nowMs, waitsOnOwner: false, selected: false,
                    keyed: false, action: {}), width: 420)
        }
        // Amber exactly when the stage says it needs a person, read from the
        // stage and not from a list of lanes.
        for n in 1...15 {
            let lane = lanes[n - 1]
            let drawn = await row(n)
            let amber = drawn["plan-lane-lane-\(n)-stage-attention"] != nil
            let gray = drawn["plan-lane-lane-\(n)-stage"] != nil
            #expect(amber == (lane.shownStage?.needsAttention == true), "lane-\(n) amber")
            #expect(gray == (lane.shownStage != nil && lane.shownStage?.needsAttention == false), "lane-\(n) gray")
        }
        #expect(lanes.filter { $0.shownStage?.needsAttention == true }.count == 3, "the fixture holds three that need a person")
        // The state line keeps the state; the stage has a line of its own.
        func second(_ n: Int) -> String {
            PlanLaneRow.second(lanes[n - 1], agent: nil, landedTrain: nil, now: store.plan.plan.nowMs, aged: false,
                state: lanes[n - 1].shownStage == nil ? nil : PlanWords.status(lanes[n - 1], stage: false))
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
        let lanes = store.plan.plan.lanes
        func stage(_ n: Int, _ width: CGFloat) async throws -> CGRect {
            let drawn = await Self.drawn(
                PlanLaneRow(
                    lane: lanes[n - 1], theme: nil, rank: nil, now: store.plan.plan.nowMs, waitsOnOwner: false, selected: false,
                    keyed: false, action: {}), width: width)
            return try #require(drawn["plan-lane-lane-\(n)-stage"])
        }
        let short = try await stage(4, 260)
        let long = try await stage(5, 260)
        #expect(long.height <= short.height + 1, "one line like the short one: \(long) \(short)")
        #expect(long.maxX <= 260, "inside the column: \(long)")
    }

    @Test("The lane page says the stage under its title, with the pull request to open, and says nothing for a silent lane")
    func lanePageStage() async throws {
        let store = try await Self.store()
        await store.plan.reload()
        let plan = store.plan.plan
        func page(_ n: Int) async -> [String: CGRect] {
            await Self.drawn(
                PlanLanePage(
                    lane: plan.lanes[n - 1], plan: plan, record: nil,
                    context: PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })),
                width: 560)
        }
        #expect(await page(4)["plan-lane-stage"] != nil)
        #expect(await page(6)["plan-lane-stage"] != nil)
        #expect(PlanWords.status(plan.lanes[5], stage: false) == "In Review", "the title keeps the state; the stage has its line")
        // A long login on the lane page: one line, then the pull request under it.
        let narrow = await Self.drawn(
            PlanLanePage(lane: plan.lanes[3], plan: plan, record: nil, context: PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })),
            width: 300)
        let stage = try #require(narrow["plan-lane-stage"])
        let wide = try #require(await page(4)["plan-lane-stage"])
        #expect(stage.height > wide.height * 1.5, "PR 31 goes under the stage when stacked: \(stage) \(wide)")
        #expect(await page(1)["plan-lane-stage"] == nil)
        #expect(await page(15)["plan-lane-stage"] == nil)
    }
}
