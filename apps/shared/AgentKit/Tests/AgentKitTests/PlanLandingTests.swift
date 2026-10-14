import Foundation
import Testing

@testable import AgentKit

// Landing (ov-522): the trains' section is Landing, never Shipping; a train is
// "Train N" with its title under it; a lane aboard a train says nothing unless
// it waits on the owner or has landed; an open train waiting for the train ahead
// says so; and the Next line never says "Nothing queued" while lanes wait.
// The shared cases (`train-lane-notes`, `next-empty`) hold the same, as words.
struct PlanLandingTests {
    private static func sections(_ name: String) throws -> PlanSections {
        let all = try PlanSectionsCasesTests.cases()
        let found = try #require(all.first { $0["name"] as? String == name })
        return try PlanSectionsCasesTests.Case(found).sections
    }

    @Test("no section is called Shipping, and the trains' section is Landing, above In Review")
    func order() throws {
        #expect(PlanSections.Kind.landing.title == "Landing")
        #expect(PlanSections.Kind.landing.emptyWords == "No train is landing.")
        #expect(PlanSections.Kind.allCases.allSatisfy { !$0.title.lowercased().contains("shipping") })
        let kinds = PlanSections.Kind.allCases
        let landing = try #require(kinds.firstIndex(of: .landing))
        #expect(kinds[landing + 1] == .inReview)
        let shown = try Self.sections("train-lane-notes").shown
        #expect(shown.starts(with: [.landing, .inReview]), "\(shown)")
    }

    @Test("a lane in landing with no train is In Review; on a train it is in the train")
    func landingWithNoTrain() throws {
        let sections = try Self.sections("train-lane-notes")
        #expect(sections.inReview.map(\.lane.name).contains("loose"))
        #expect(!sections.trains.flatMap(\.lanes).map(\.name).contains("loose"))
        #expect(sections.trains.flatMap(\.lanes).map(\.name).contains("aboard"))
        #expect(!sections.inReview.map(\.lane.name).contains("aboard"))
    }

    @Test("an open train waiting for the train ahead says so in place of its first step")
    func waitingForThePreviousTrain() throws {
        let sections = try Self.sections("train-lane-notes")
        let open = try #require(sections.trains.first { $0.train.name == "integ-6" })
        #expect(open.phase == .waitingForPreviousTrain)
        #expect(PlanLadder.compact(open.phase) == "Waiting for the Previous Train \u{00B7} step 1 of 3")
        #expect(PlanLadder.label(0, current: PlanLadder.step(open.phase), phase: open.phase) == "Waiting for the Previous Train")
        #expect(open.phase.sentence == "waiting for the previous train")
        #expect(open.phase.explanation.hasSuffix("."))
        #expect(!open.phase.needsAttention)
        #expect(TrainPhase.waitingForPreviousTrain.progress < TrainPhase.integrating.progress, "an open train is behind one that is being checked")
        let wire = try JSONDecoder().decode(TrainPhase.self, from: Data("\"waiting_for_previous_train\"".utf8))
        #expect(wire == .waitingForPreviousTrain)
    }

    @Test("a lane aboard a train says nothing, except to wait on the owner or once landed; none says it holds the train")
    func laneNotes() throws {
        let lanes = try #require(try Self.sections("train-lane-notes").trains.first).lanes
        func note(_ name: String) -> String? { lanes.first { $0.name == name }?.note }
        #expect(note("aboard") == nil, "'Ready' said nothing: ready for what?")
        #expect(note("holder") == nil, "a lane fixing aboard is simply aboard")
        #expect(note("gone") == "Landed \u{00B7} 3f78754f")
        #expect(note("blocked") == "Needs you")
        #expect(lanes.first { $0.name == "blocked" }?.noteNeedsAttention == true)
        #expect(lanes.first { $0.name == "aboard" }?.noteNeedsAttention == false)
        #expect(lanes.compactMap(\.note).allSatisfy { !$0.contains("holding") })
    }

    @Test("the sidebar's Landing group holds the trains as Train N with their titles, and their lanes inside")
    func sidebar() throws {
        let sections = try Self.sections("train-lane-notes")
        let case_ = try #require(try PlanSectionsCasesTests.cases().first { $0["name"] as? String == "train-lane-notes" })
        let tree = LaneTree(plan: try PlanSectionsCasesTests.Case(case_).plan, sections: sections)
        #expect(tree.groups.map(\.title) == ["Landing", "In Review"])
        let landing = try #require(tree.groups.first)
        #expect(landing.lanes.isEmpty && landing.trains.count == 2)
        let train = landing.trains[0]
        #expect(train.label == "Train 5" && train.subtitle == "People on a shared runner; sign in on the web")
        #expect(train.lanes.map(\.name) == ["aboard", "holder", "gone", "blocked"])
        #expect(landing.trains.map(\.label) == ["Train 5", "Train 6"])
        #expect(train.lanes.first { $0.name == "aboard" }?.state == "")
        #expect(train.lanes.first { $0.name == "gone" }?.state == "Landed", "the commit stays on the card; it would take the title's room here")
        #expect(tree.cards(of: "l-holder").map(\.key) == ["ov-16"], "a lane inside a train still answers for its cards")
        #expect(tree.groups.allSatisfy { group in
            if case .landing = group.kind { return true }
            return group.trains.isEmpty
        }, "no train-named group sits beside Building or Waiting")
    }

    @Test("a train's title is its subtitle, without the runner's own 'Train N ·' in front")
    func subtitle() throws {
        let sections = try Self.sections("long-train-title")
        let first = try #require(sections.trains.first?.train)
        #expect(first.label == "Train 89")
        #expect(first.subtitle?.hasPrefix("The agents table") == true || first.subtitle?.hasPrefix("the agents table") == true, "\(first.subtitle ?? "-")")
        #expect(first.subtitle?.contains("Train 89") == false)
    }

    @Test("the Next line counts the lanes that wait while none is ranked, and says nothing queued only when none waits")
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

    @Test("the sidebar lists no agent that has finished: a runner that lists agents lists the ones that exist")
    func sidebarAgentsStillExist() throws {
        let found = try #require(try PlanSectionsCasesTests.cases().first { $0["name"] as? String == "train-lane-notes" })
        var plan = try PlanSectionsCasesTests.Case(found).plan
        func agents(_ plan: PlanModel) -> [String] {
            LaneTree(plan: plan).groups.flatMap(\.lanes).first { $0.name == "loose" }?.agents.map(\.who) ?? ["no lane"]
        }
        // The lane records a finished reviewer and an open builder, and the runner lists agents, none of them this lane's.
        #expect(plan.agentsSpend != nil)
        #expect(agents(plan) == [], "a finished agent stayed in the sidebar: \(agents(plan))")
        // Even with no agent alive at all, a runner that totals the day lists the ones that exist.
        plan.agents = []
        #expect(agents(plan) == [], "\(agents(plan))")
        // An older runner: only what the orchestrator recorded and has not ended.
        plan.agentsSpend = nil
        #expect(agents(plan) == ["Builder \u{00B7} Sonnet"], "\(agents(plan))")
    }

    @Test("no train word says holding, 'this Mac' or 'Waits for you', and the explanations tell one story")
    func plainTrainWords() throws {
        let sections = try Self.sections("held-train")
        let held = try #require(sections.trains.first?.held)
        #expect(held == "Waiting for slow \u{00B7} 1 h")
        var lines: [String] = []
        for phase in TrainPhase.allCases { lines += [phase.word, phase.sentence, phase.explanation] }
        lines += PlanLadder.explanations
        lines += [PlanWords.trainExplanation, held]
        let said = lines.joined(separator: " ")
        for old in ["Holding", "holding", "this Mac", "Waits for you"] { #expect(!said.contains(old), "\(old)") }
        // Checking here, then CI, then landed: the CI step is pushed, not yet landed; ready to land is not on main yet.
        #expect(PlanLadder.explanations[1] == "Pushed, with CI running its checks.")
        #expect(PlanLadder.explanations[2].contains("has landed on main"))
        #expect(TrainPhase.readyToLand.explanation == "Every check passed, so it's ready to land.")
    }

    @Test("a train in the Mac sidebar leads its second line with where it stands, so a long phase never crushes its name")
    func sidebarTrainRow() throws {
        let found = try #require(try PlanSectionsCasesTests.cases().first { $0["name"] as? String == "train-lane-notes" })
        let outline = SidebarOutline.build(OneTreeInput(tasks: [], plan: try PlanSectionsCasesTests.Case(found).plan))
        let landing = try #require(outline.lanes.first { $0.title == "Landing" })
        let open = try #require(landing.children.first { $0.title == "Train 6" })
        #expect(open.detail.isEmpty, "a long phase at the trailing edge squeezed the title to a letter")
        #expect(open.caption == "Waiting for the Previous Train")
        let first = try #require(landing.children.first { $0.title == "Train 5" })
        #expect(first.caption == "Waiting for Lanes \u{00B7} People on a shared runner; sign in on the web")
    }
}
