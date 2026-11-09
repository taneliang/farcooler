import Foundation
import Testing

@testable import AgentKit

// Landing (ov-522): the landings' section is Landing, never Shipping; a landing
// is "Landing N" with its title under it; a ticket aboard a landing says nothing
// unless it waits on the owner or has landed; an open landing waiting for the
// landing ahead says so; and the Next line never says "Nothing queued" while
// tickets wait. The shared cases (`landing-ticket-notes`, `next-empty`) hold the
// same, as words; their names are identifiers Android and the Mac read by. The
// sections themselves are the Rust core's (ov-1016) and its tests' to pin;
// these are what still reads them here: the band, the sidebar and the words.
struct PlanLandingTests {
    private static func view(_ name: String) throws -> PlanView {
        let all = try PlanSectionsCasesTests.cases()
        let found = try #require(all.first { $0["name"] as? String == name })
        return try #require(try PlanSectionsCasesTests.Case(found).view)
    }

    @Test("the sidebar's Landing group holds the landings as Landing N with their titles, and their tickets inside")
    func sidebar() throws {
        let view = try Self.view("landing-ticket-notes")
        let case_ = try #require(try PlanSectionsCasesTests.cases().first { $0["name"] as? String == "landing-ticket-notes" })
        let tree = TicketTree(plan: try PlanSectionsCasesTests.Case(case_).plan, view: view)
        #expect(tree.groups.map(\.title) == ["Landing", "In Review"], "ov-14, in a fix round, is In Review")
        let landing = try #require(tree.groups.first)
        #expect(landing.tickets.isEmpty && landing.landings.count == 2)
        let first = landing.landings[0]
        #expect(first.label == "Landing 5" && first.subtitle == "People on a shared runner; sign in on the web")
        #expect(first.tickets.map(\.key) == ["ov-15", "ov-16", "ov-17", "ov-18"])
        #expect(landing.landings.map(\.label) == ["Landing 5", "Landing 6"])
        #expect(first.tickets.first { $0.key == "ov-15" }?.state == "")
        #expect(first.tickets.first { $0.key == "ov-17" }?.state == "Landed", "the commit stays on the card; it would take the title's room here")
        #expect(tree.cards(of: "t-ov-16").map(\.key) == ["ov-16"], "a ticket inside a landing still answers for its card")
        #expect(tree.groups.allSatisfy { group in
            if case .landing = group.kind { return true }
            return group.landings.isEmpty
        }, "no landing-named group sits beside Building or Waiting")
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

    @Test("no landing word says holding, 'this Mac' or 'Waits for you', and the explanations tell one story")
    func plainLandingWords() throws {
        let view = try Self.view("landing-ticket-notes")
        let card = try #require(view.landings.first)
        let held = card.spoken(ci: nil, now: view.now)
        #expect(!held.contains("Holding") && !held.contains("holding"), "\(held)")
        #expect(card.tickets.compactMap(\.note).allSatisfy { !$0.lowercased().contains("holding") }, "\(card.tickets.map(\.note))")
        var lines: [String] = []
        for landing in view.landings { lines += [landing.phaseWord, landing.phaseSentence, landing.explanation] }
        lines += card.ladder.steps.flatMap { [$0.label, $0.explanation] }
        lines += [view.section(.landing)?.help ?? ""]
        let said = lines.joined(separator: " ")
        for old in ["Holding", "holding", "this Mac", "Waits for you", "lane", "Lane", "train", "Train"] { #expect(!said.contains(old), "\(old)") }
        // Checking here, then CI, then landed: the CI step is pushed, not yet landed; ready to land is not on main yet.
        #expect(card.ladder.steps[1].explanation == "Pushed, with CI running its checks.")
        #expect(card.ladder.steps[2].explanation.contains("has landed on main"))
    }

    @Test("a landing in the Mac sidebar leads its second line with where it stands, so a long phase never crushes its name")
    func sidebarLandingRow() throws {
        let found = try #require(try PlanSectionsCasesTests.cases().first { $0["name"] as? String == "landing-ticket-notes" })
        let outline = SidebarOutline.build(OneTreeInput(tasks: [], plan: try PlanSectionsCasesTests.Case(found).plan))
        let landing = try #require(outline.tickets.first { $0.title == "Landing" })
        let open = try #require(landing.children.first { $0.title == "Landing 6" })
        #expect(open.detail.isEmpty, "a long phase at the trailing edge squeezed the title to a letter")
        #expect(open.caption == "Waiting for the Earlier Landing")
        let first = try #require(landing.children.first { $0.title == "Landing 5" })
        #expect(first.caption == "Waiting for Tickets \u{00B7} People on a shared runner; sign in on the web")
    }
}
