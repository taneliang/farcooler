import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// The plan column on the Mac (ov-867, C10f): the real `PlanHome` over
/// `PlanColumnScenario`'s plan, read through the client's own command runner.
/// The rules are AgentKit's (`PlanSectionsColumnTests`); this holds the view
/// to them: the order, the tickets under their agent, nothing truncated, and
/// no Stop, Assign or cost.
@MainActor
@Suite(.serialized)
struct PlanColumnViewTests {
    static func store() async throws -> TaskBoardStore {
        let calls = PlanViewTests.Calls()
        calls.plan = try PlanColumnScenario.data()
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        return store
    }

    static func drawn(width: CGFloat) async throws -> [String: CGRect] {
        let store = try await Self.store()
        #expect(PlanColumnScenario.shown(store.plan.plan), "the scenario read")
        return await PlanThemePageTests.frames(
            PlanHome(board: store, needsYou: PlanNeedsYou(), onOpen: { _ in }), width: width, height: 4000)
    }

    @Test("Facts, Needs You, Initiatives, the pipeline by stage, then the footer, in that order")
    func order() async throws {
        let drawn = try await Self.drawn(width: 720)
        let order = [
            "plan-facts", "plan-needs-you", "plan-initiatives", "plan-pipeline", "plan-stage-stuck", "plan-stage-building",
            "plan-stage-reviewing", "plan-stage-landing", "plan-footer",
        ]
        let frames = try order.map { try #require(drawn[$0], "\($0) in \(drawn.keys.sorted())") }
        for (a, b) in zip(order.indices, order.indices.dropFirst()) where !["plan-pipeline"].contains(order[a]) {
            #expect(frames[a].maxY <= frames[b].minY + 0.5, "\(order[a]) then \(order[b])")
        }
        #expect(drawn["plan-fact-stuck"] != nil && drawn["plan-fact-nextLanding"] != nil && drawn["plan-fact-done"] != nil)
    }

    @Test("An agent's tickets are indented under its header, inside its card, each with its own initiative")
    func ticketsUnderTheirAgent() async throws {
        let drawn = try await Self.drawn(width: 720)
        let builder = try #require(drawn["plan-agent-agent:b3"])
        let chat = try #require(drawn["plan-ticket-ov-881"])
        let task = try #require(drawn["plan-ticket-ov-882"])
        #expect(chat.minY >= builder.maxY - 0.5 && task.minY >= chat.maxY - 0.5, "the tickets follow the header")
        let glyph = try #require(drawn["plan-agent-glyph-agent:b3"])
        let key = try #require(drawn["plan-ticket-key-ov-881"])
        #expect(key.minX >= glyph.maxX, "the tickets are indented past the role glyph: \(glyph) \(key)")
        let other = try #require(drawn["plan-ticket-key-ov-882"])
        #expect(abs(key.width - other.width) < 0.5 && abs(key.minX - other.minX) < 0.5, "keys share one column and width")
        #expect(drawn.keys.allSatisfy { !$0.hasPrefix("plan-ticket-mark-") }, "no dot")
        let lander = try #require(drawn["plan-agent-agent:i1"])
        let landing = try #require(drawn["plan-stage-landing"])
        #expect(landing.contains(lander), "the lander lands the landing")
    }

    @Test("At a 720-pt column, as at 1,440 wide, no ticket title is cut: a long one wraps")
    func nothingTruncates() async throws {
        let wide = try await Self.drawn(width: 720)
        let narrow = try await Self.drawn(width: 420)
        let long = "plan-ticket-ov-880"
        let one = try #require(wide["plan-ticket-ov-881"]).height
        #expect(try #require(wide[long]).height <= one + 1, "the long title fits on one line at 720")
        #expect(try #require(narrow[long]).height > one + 4, "and wraps, rather than truncating, at 420")
    }

    @Test("The column offers no Stop, Stop All, Assign or cost: you say those to the orchestrator")
    func nothingToActOnAnAgent() async throws {
        let drawn = try await Self.drawn(width: 720)
        let banned = ["stop", "assign", "cost", "spend"]
        #expect(drawn.keys.allSatisfy { key in !banned.contains { key.lowercased().contains($0) } }, "\(drawn.keys.sorted())")
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<3 { root.deleteLastPathComponent() }
        let source = try String(
            contentsOf: root.appendingPathComponent("Sources/FarCooler/Plan/PlanColumnView.swift"), encoding: .utf8)
        for word in ["\"Stop", "\"Assign", "Divider()", "PlanCost"] { #expect(!source.contains(word), "\(word) is in the column") }
    }

    /// A view hosted in a key window, to read where it drew and to click it.
    @MainActor private final class Clicking {
        let seen = NavigatorFilterTests.Seen()
        var host: NSHostingView<AnyView>!
        var window: NavigatorFilterTests.KeyWindow!
        static let height: CGFloat = 1600

        func settle() async {
            for _ in 0..<10 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }

        func click(_ id: String) -> Bool {
            guard let frame = seen.views[id] else { return false }
            let at = NSPoint(x: frame.midX, y: Self.height - frame.midY)
            for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                window.sendEvent(
                    NSEvent.mouseEvent(
                        with: type, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                        windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!)
            }
            return true
        }
    }

    private static func host(_ view: some View) async -> Clicking {
        let clicking = Clicking()
        let seen = clicking.seen
        clicking.host = NSHostingView(
            rootView: AnyView(
                view
                    .frame(width: 720, height: Clicking.height, alignment: .topLeading)
                    .environment(\.gridProbing, true)
                    .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                        GeometryReader { proxy in
                            let _ = seen.views = Dictionary(
                                probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                            Color.clear
                        }
                    }))
        clicking.window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 720, height: Clicking.height), styleMask: [.borderless],
            backing: .buffered, defer: false)
        clicking.window.isReleasedWhenClosed = false
        clicking.window.contentView = clicking.host
        TestWindow.show(clicking.window)
        await clicking.settle()
        return clicking
    }

    @Test("The \"N more\" line under an agent opens its stage's tickets, and a row there opens its task")
    func moreOpensItsList() async throws {
        let store = try await Self.store()
        let column = PlanColumn(
            plan: store.plan.plan, statuses: store.board.statuses, titles: PlanHome.titles(store.board))
        let reviewing = try #require(column.pipeline.first { $0.stage == .reviewing })
        let card = try #require(reviewing.agents.first { $0.more > 0 }, "the scenario has a reviewer with more waiting")
        var opened: [PlanPage] = []
        let pipeline = await Self.host(
            PlanPipelineSection(column: column, actions: PlanColumnActions(), onOpen: { opened.append($0) }))
        defer { pipeline.window.close() }
        #expect(pipeline.click("plan-more-\(card.id)"), "the more line is a control: \(pipeline.seen.views.keys.sorted())")
        #expect(opened == [.stage("reviewing")], "it opens the stage list: \(opened) \(String(describing: pipeline.seen.views["plan-more-\(card.id)"]))")

        let rows = Dictionary(
            store.board.rows.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        var tasks: [String] = []
        let page = await Self.host(
            PlanStageListPage(
                plan: store.plan, stage: "reviewing",
                context: PlanPageContext(rows: rows, onTask: { tasks.append($0.id) }, onOpen: { opened.append($0) })))
        defer { page.window.close() }
        let first = try #require(reviewing.unattended.first)
        #expect(
            Set(reviewing.unattended.map { "plan-ticket-\($0.key)" }).isSubset(of: Set(page.seen.views.keys)),
            "every ticket nobody is on is listed: \(page.seen.views.keys.sorted())")
        let clicked = page.click("plan-ticket-\(first.key)")
        #expect(clicked)
        #expect(tasks == [first.task] || opened.contains(.ticket(first.task)), "a row opens its ticket: \(tasks) \(opened)")
    }
}
