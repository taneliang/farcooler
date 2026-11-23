import Foundation
import Testing

@testable import AgentKit

// The sidebar and glance consume the shared Plan view.
struct PlanLandingTests {
    private static func view(_ name: String) throws -> WirePlanView {
        let all = try PlanSectionsCasesTests.cases()
        let found = try #require(all.first { $0["name"] as? String == name })
        return try #require(try PlanSectionsCasesTests.Case(found).view)
    }

    @Test("the sidebar omits historical landing rows")
    func sidebar() throws {
        let view = try Self.view("landing-ticket-notes")
        let case_ = try #require(try PlanSectionsCasesTests.cases().first { $0["name"] as? String == "landing-ticket-notes" })
        let tree = TicketTree(plan: try PlanSectionsCasesTests.Case(case_).plan, view: view)
        #expect(view.landings.isEmpty)
        #expect(tree.groups.allSatisfy { $0.landings.isEmpty })
    }

    @Test("the Next line counts the tickets that wait while none is ranked, and says nothing queued only when none waits")
    func nextLine() throws {
        let waiting = try Self.view("next-empty")
        #expect(waiting.waiting(.next).isEmpty && waiting.waitingCount == 3)
        let said = PlanGlanceSentence(view: waiting)
        #expect(said.nextLine == "3 waiting, none ranked")
        #expect(said.text.hasSuffix("Next: 3 waiting, none ranked."))
        let nothing = PlanGlanceSentence(view: try Self.view("nothing"))
        #expect(nothing.nextLine == PlanGlanceSentence.nothingQueued)
        let ranked = PlanGlanceSentence(view: try Self.view("next-ranked"))
        #expect(!ranked.nextLine.contains("none ranked"))
    }

    @Test("the sidebar lists no agent that has finished: a ticket lists the agents that exist and work it")
    func sidebarAgentsStillExist() throws {
        let found = try #require(try PlanSectionsCasesTests.cases().first { $0["name"] as? String == "landing-ticket-notes" })
        var plan = try PlanSectionsCasesTests.Case(found).plan
        func agents(_ plan: PlanModel) -> [String] {
            TicketTree(plan: plan).groups.flatMap(\.tickets).first { $0.key == "ov-11" }?.agents.map(\.who) ?? ["no ticket"]
        }
        // The ticket's work records a finished reviewer and an open builder; the runner lists only the builder, which exists.
        #expect(plan.agentsSpend != nil)
        #expect(plan.line(key: "ov-11")?.work.map(\.role) == [.review, .build])
        #expect(agents(plan) == ["Builder \u{00B7} sonnet"], "only the agent that exists: \(agents(plan))")
        // Once it is gone from the list, the work's record alone lists nothing.
        plan.agents.removeAll { $0.id == "agent:a-open" }
        #expect(agents(plan) == [], "a finished agent stayed in the sidebar: \(agents(plan))")
    }

}
