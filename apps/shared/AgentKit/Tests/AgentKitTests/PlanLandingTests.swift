import Foundation
import Testing

@testable import AgentKit

// Landing (ov-522): the landings' section is Landing, never Shipping; a landing
// is "Landing N" with its title under it; a ticket aboard a landing says nothing
// unless it waits on the owner or has landed; an open landing waiting for the
// landing ahead says so; and the Next line never says "Nothing queued" while
// tickets wait. The shared cases (`landing-ticket-notes`, `next-empty`) hold the
// same, as words; their names are identifiers Android and the Mac read by.
struct PlanLandingTests {
    private static func sections(_ name: String) throws -> PlanSections {
        let all = try PlanSectionsCasesTests.cases()
        let found = try #require(all.first { $0["name"] as? String == name })
        return try PlanSectionsCasesTests.Case(found).sections
    }

    @Test("no section is called Shipping, and the landings' section is Landing, above In Review")
    func order() throws {
        #expect(PlanSections.Kind.landing.title == "Landing")
        #expect(PlanSections.Kind.landing.emptyWords == "Nothing is landing right now.")
        #expect(PlanSections.Kind.allCases.allSatisfy { !$0.title.lowercased().contains("shipping") })
        let kinds = PlanSections.Kind.allCases
        let landing = try #require(kinds.firstIndex(of: .landing))
        #expect(kinds[landing + 1] == .inReview)
        let shown = try Self.sections("landing-ticket-notes").shown
        #expect(shown.starts(with: [.landing, .inReview]), "\(shown)")
    }

    @Test("a ticket in the landing stage on no landing is In Review; on a landing it is in the landing")
    func landingWithNoLanding() throws {
        let sections = try Self.sections("landing-ticket-notes")
        #expect(sections.inReview.map(\.line.key).contains("ov-11"))
        #expect(!sections.landings.flatMap(\.tickets).map(\.key).contains("ov-11"))
        #expect(sections.landings.flatMap(\.tickets).map(\.key).contains("ov-15"))
        #expect(!sections.inReview.map(\.line.key).contains("ov-15"))
    }

    @Test("an open landing waiting for the landing ahead says so in place of its first step")
    func waitingForThePreviousLanding() throws {
        let sections = try Self.sections("landing-ticket-notes")
        let open = try #require(sections.landings.first { $0.landing.number == 6 })
        #expect(open.phase == .waitingForPreviousLanding)
        #expect(PlanLadder.compact(open.phase) == "Waiting for the Earlier Landing \u{00B7} step 1 of 3")
        #expect(PlanLadder.label(0, current: PlanLadder.step(open.phase), phase: open.phase) == "Waiting for the Earlier Landing")
        #expect(open.phase.sentence == "waiting for the earlier landing")
        #expect(open.phase.explanation.hasSuffix("."))
        #expect(!open.phase.needsAttention)
        #expect(LandingPhase.waitingForPreviousLanding.progress < LandingPhase.integrating.progress, "an open landing is behind one that is being checked")
        let wire = try JSONDecoder().decode(LandingPhase.self, from: Data("\"waiting_for_previous_landing\"".utf8))
        #expect(wire == .waitingForPreviousLanding)
        // The old words are a phase this build has no word for.
        #expect(try JSONDecoder().decode(LandingPhase.self, from: Data("\"waiting_for_previous_train\"".utf8)) == .unknown)
    }

    @Test("the collecting landing with nothing ahead but main's own checks says so, on the first step, behind a landing being checked")
    func waitingForMain() throws {
        let plan = try PlanModelTests.plan(
            themes: [], lines: [],
            landings: [PlanModelTests.landing(7, "open", phase: "waiting_for_main", since: 5, phaseSince: 9)])
        let card = try #require(PlanSections(plan: plan).landings.first)
        #expect(card.phase == .waitingForMain)
        #expect(card.phase.word == "Waiting for Main's Checks to Pass")
        #expect(card.phase.sentence == "waiting for main's checks to pass")
        #expect(PlanLadder.compact(card.phase) == "Waiting for Main's Checks to Pass \u{00B7} step 1 of 3")
        #expect(PlanLadder.label(0, current: PlanLadder.step(card.phase), phase: card.phase) == "Waiting for Main's Checks to Pass")
        #expect(PlanLadder.explanation(0, current: 0, phase: card.phase) == card.phase.explanation)
        #expect(card.phase.explanation.hasSuffix(".") && !card.phase.needsAttention)
        #expect(card.spoken(ci: nil, now: 1).contains("Waiting for Main's Checks to Pass \u{00B7} step 1 of 3"))
        #expect(LandingPhase.waitingForMain.progress == LandingPhase.waitingForPreviousLanding.progress)
        #expect(LandingPhase.waitingForMain.progress < LandingPhase.integrating.progress)
        let wire = try JSONDecoder().decode(LandingPhase.self, from: Data("\"waiting_for_main\"".utf8))
        #expect(wire == .waitingForMain)
    }

    @Test("a ticket aboard a landing says nothing, except to wait on the owner or once landed; none says it holds the landing")
    func ticketNotes() throws {
        let tickets = try #require(try Self.sections("landing-ticket-notes").landings.first).tickets
        func note(_ key: String) -> String? { tickets.first { $0.key == key }?.note }
        #expect(note("ov-15") == nil, "'Ready' said nothing: ready for what?")
        #expect(note("ov-16") == nil, "a ticket fixing aboard is simply aboard")
        #expect(note("ov-17") == "Landed \u{00B7} 3f78754f")
        #expect(note("ov-18") == "Needs you")
        #expect(tickets.first { $0.key == "ov-18" }?.noteNeedsAttention == true)
        #expect(tickets.first { $0.key == "ov-15" }?.noteNeedsAttention == false)
        #expect(tickets.compactMap(\.note).allSatisfy { !$0.contains("holding") })
    }

    @Test("the sidebar's Landing group holds the landings as Landing N with their titles, and their tickets inside")
    func sidebar() throws {
        let sections = try Self.sections("landing-ticket-notes")
        let case_ = try #require(try PlanSectionsCasesTests.cases().first { $0["name"] as? String == "landing-ticket-notes" })
        let tree = TicketTree(plan: try PlanSectionsCasesTests.Case(case_).plan, sections: sections)
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

    @Test("a landing's title is its subtitle, without the runner's own 'Landing N ·' in front")
    func subtitle() throws {
        let sections = try Self.sections("long-landing-title")
        let first = try #require(sections.landings.first?.landing)
        #expect(first.label == "Landing 89")
        #expect(first.subtitle?.hasPrefix("The agents table") == true, "\(first.subtitle ?? "-")")
        #expect(first.subtitle?.contains("Landing 89") == false)
        let second = try #require(sections.landings.last?.landing)
        #expect(second.label == "Landing 901" && second.subtitle?.hasPrefix("Login hotfix") == true, "a title that doesn't repeat its number stays whole")
    }

    @Test("the Next line counts the tickets that wait while none is ranked, and says nothing queued only when none waits")
    func nextLine() throws {
        let waiting = try Self.sections("next-empty")
        #expect(waiting.waiting.next.isEmpty && waiting.waiting.count == 3)
        let said = PlanGlanceSentence(sections: waiting)
        #expect(said.nextLine == "3 waiting, none ranked")
        #expect(said.text.hasSuffix("Next: 3 waiting, none ranked."))
        let nothing = PlanGlanceSentence(sections: try Self.sections("nothing"))
        #expect(nothing.nextLine == PlanGlanceSentence.nothingQueued)
        let ranked = PlanGlanceSentence(sections: try Self.sections("next-ranked"))
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
        #expect(agents(plan) == ["Builder \u{00B7} Sonnet"], "only the agent that exists: \(agents(plan))")
        // Once it is gone from the list, the work's record alone lists nothing.
        plan.agents.removeAll { $0.id == "agent:a-open" }
        #expect(agents(plan) == [], "a finished agent stayed in the sidebar: \(agents(plan))")
    }

    @Test("no landing word says holding, 'this Mac' or 'Waits for you', and the explanations tell one story")
    func plainLandingWords() throws {
        let sections = try Self.sections("landing-ticket-notes")
        let card = try #require(sections.landings.first)
        let held = card.spoken(ci: nil, now: sections.now)
        #expect(!held.contains("Holding") && !held.contains("holding"), "\(held)")
        #expect(card.tickets.compactMap(\.note).allSatisfy { !$0.lowercased().contains("holding") }, "\(card.tickets.map(\.note))")
        var lines: [String] = []
        for phase in LandingPhase.allCases { lines += [phase.word, phase.sentence, phase.explanation] }
        lines += PlanLadder.explanations
        lines += [PlanWords.landingExplanation]
        let said = lines.joined(separator: " ")
        for old in ["Holding", "holding", "this Mac", "Waits for you", "lane", "Lane", "train", "Train"] { #expect(!said.contains(old), "\(old)") }
        // Checking here, then CI, then landed: the CI step is pushed, not yet landed; ready to land is not on main yet.
        #expect(PlanLadder.explanations[1] == "Pushed, with CI running its checks.")
        #expect(PlanLadder.explanations[2].contains("has landed on main"))
        #expect(LandingPhase.readyToLand.explanation == "Every check passed, so it's ready to land.")
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
