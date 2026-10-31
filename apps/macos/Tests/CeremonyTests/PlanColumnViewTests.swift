import AgentKit
import AppKit
import SwiftUI
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
        #expect(chat.minX > builder.minX, "the tickets are indented: \(builder) \(chat)")
        let lander = try #require(drawn["plan-agent-agent:i1"])
        let landing = try #require(drawn["plan-stage-landing"])
        #expect(landing.contains(lander), "the integrator lands the train")
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
}
