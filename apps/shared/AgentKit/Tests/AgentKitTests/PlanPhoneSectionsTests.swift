import Foundation
import Testing

@testable import AgentKit

/// What `PlanSectionsUITests` pinned by launching the app and scrolling, as
/// values over the same fixture (`plan-phones-ov503.json`, ov-538): the order of
/// the sections, the band's words, a train and the words it speaks, Waiting's
/// groups, the fold, the agents, the themes and the two pushed lists.
struct PlanPhoneSectionsTests {
    typealias F = PlanPhoneFixtures

    // MARK: The band

    @Test("the band says Now and Next in two lines, and what needs the owner as its one mark")
    func theBand() throws {
        let sections = try F.sections()
        let band = PlanGlanceSentence(sections: sections)
        let spoken = band.spoken(needsYou: sections.needsYou)
        #expect(spoken.contains("Train 86 checking on CI"), "\(spoken)")
        #expect(spoken.contains(" building") && spoken.contains(" in review"), "\(spoken)")
        #expect(spoken.contains("Next: A real window capture on the front page, then Onboarding"), "\(spoken)")
        #expect(spoken.hasSuffix(" needs you."), "\(spoken)")
    }

    @Test("at the sheet's medium detent the band says Now only; at large it says Next as well")
    func theBandAtMedium() throws {
        let sections = try F.sections()
        let band = PlanGlanceSentence(sections: sections)
        let medium = band.spoken(needsYou: 2, nowOnly: true)
        #expect(medium.contains("Train 86 checking on CI"), "\(medium)")
        #expect(!medium.contains("Next:"), "the band says Next at the medium detent: \(medium)")
        #expect(medium.hasSuffix("2 need you."), "\(medium)")
        #expect(band.spoken(needsYou: 2, nowOnly: false).contains("Next:"))
        #expect(band.spoken(needsYou: 0, nowOnly: true) == "\(band.nowLine).", "nothing needs you, so nothing is added")
        #expect(band.spoken(needsYou: 1) .hasSuffix("1 needs you."))
    }

    // MARK: The sections

    @Test("the sections come in the owner's order: Landing, In Review, Building, Waiting, On a Landed Train, Agents, Themes")
    func order() throws {
        let sections = try F.sections()
        #expect(
            sections.shown == [.needsYou, .landing, .inReview, .building, .waiting, .onLandedTrain, .agents, .themes],
            "\(sections.shown)")
        #expect(PlanSections.Kind.allCases == [.needsYou, .landing, .inReview, .building, .waiting, .onLandedTrain, .agents, .themes])
        #expect(PlanSections.Kind.allCases.map(\.title) == [
            "Needs You", "Landing", "In Review", "Building", "Waiting", "On a Landed Train", "Agents", "Themes",
        ])
    }

    @Test("a train is Train N with its title under it, where it stands on the ladder, and a held one says what holds it")
    func theLadderAndTheHeldTrain() throws {
        let sections = try F.sections()
        let now = sections.now
        func card(_ name: String) throws -> PlanSections.TrainCard { try #require(sections.trains.first { $0.train.name == name }) }
        let ready = try card("integ-86").spoken(ci: nil, now: now)
        #expect(ready.hasPrefix("Train 86, Teams on a shared runner"), "\(ready)")
        #expect(ready.contains("Checking on CI \u{00B7} step 2 of 3"), "\(ready)")
        #expect(ready.contains("3f78754f"), "the commit isn't said: \(ready)")
        #expect(ready.contains("Integrator \u{00B7} Opus"), "\(ready)")
        #expect(ready.contains("The team directory; the iPhone composer"), "\(ready)")
        #expect(!ready.contains("integ-86"), "the slug is for a person who needs it, not in the text: \(ready)")

        let held = try card("integ-89").spoken(ci: nil, now: now)
        #expect(held.contains("Waiting for Lanes \u{00B7} step 1 of 3"), "\(held)")
        #expect(held.contains("Waiting for ios-overwrite"), "the hold isn't said: \(held)")

        #expect(try card("integ-84").spoken(ci: nil, now: now).contains("Stalled \u{00B7} step 1 of 3"))
    }

    @Test("a lane aboard a train says nothing of its own state, and none says it holds the train")
    func aLaneAboardSaysNothing() throws {
        let sections = try F.sections()
        let ready = try #require(sections.trains.first { $0.train.name == "integ-86" })
        let teamDir = try #require(ready.lanes.first { $0.name == "team-dir" })
        #expect(teamDir.note == nil, "a lane aboard says nothing: ready for what?")
        let held = try #require(sections.trains.first { $0.train.name == "integ-89" })
        let holder = try #require(held.lanes.first { $0.name == "ios-overwrite" })
        #expect(holder.note == nil && !(holder.standing.word.isEmpty), "its round isn't said aboard")
        #expect(holder.note?.contains("round 2") != true)
    }

    @Test("a long press explains what a train is, then each step of its ladder, the one it stands on in the phase's words")
    func explanation() throws {
        let sections = try F.sections()
        let train = try #require(sections.trains.first { $0.train.name == "integ-86" })
        let said = train.explanation
        #expect(said.first == "A train is a batch of reviewed lanes that land on main together, after one round of checks.")
        #expect(said.count == 1 + PlanLadder.steps.count)
        #expect(said.contains("Checking Here: The train's full test run, on the runner."), "\(said)")
        #expect(PlanWords.trainExplanation == said[0])
    }

    @Test("Waiting is ranked, then not ranked, and each group is named once something else shares the section")
    func waitingGroups() throws {
        let waiting = try F.sections().waiting
        #expect(waiting.groups.map(\.id) == ["plan-next-up", "plan-not-ranked", "plan-waiting-on-you"])
        #expect(waiting.groups.map(\.title) == ["Next Up", "Not Ranked", "Waiting on You"])
        #expect(waiting.namesGroups)
        let next = try #require(waiting.groups.first)
        #expect(next.rows.first?.lane.name == "hero-shot")
        #expect(next.rows.first?.lane.reason == "Unblocked once the site is one project")
        #expect(next.rank(at: 0) == 1 && next.rank(at: 1) == 2, "the first ranked lane is numbered 1")
        #expect(waiting.groups[1].rank(at: 0) == nil && waiting.groups[2].rank(at: 0) == nil)
        #expect(waiting.groups[2].attention && !next.attention)
        #expect(waiting.onYou.map(\.lane.name) == ["web-site"], "the lane whose card needs a decision is Waiting on You")
        #expect(!waiting.next.map(\.lane.name).contains("web-site"))
    }

    @Test("a group alone in Waiting is not named")
    func aLoneGroupIsNotNamed() throws {
        let plan = try PlanModelTests.plan(
            themes: [], lanes: [PlanModelTests.lane("a", "queued", cards: [], rank: 1)], order: ["a"])
        let waiting = PlanSections(plan: plan).waiting
        #expect(waiting.groups.map(\.id) == ["plan-next-up"] && !waiting.namesGroups)
    }

    @Test("lanes on a landed train are one folded section that says how many")
    func landedTrainFolds() throws {
        let sections = try F.sections()
        #expect(PlanSections.Kind.onLandedTrain.startsFolded)
        #expect(PlanSections.Kind.allCases.filter(\.startsFolded) == [.onLandedTrain], "only this section starts folded")
        #expect(sections.onLandedTrain.map(\.lane.name) == ["old-queued"])
        #expect(sections.count(.onLandedTrain) == 1)
        #expect(sections.onLandedTrain.first?.landedTrain == "Train 85")
    }

    @Test("agents are who, for what and how many tokens, working ones first; one that finished is not listed")
    func agents() throws {
        let sections = try F.sections()
        let fleet = sections.fleet
        #expect(fleet.header(locale: F.en).contains("working"))
        let builder = try #require(fleet.agents.first { $0.id == "pane:b1" })
        let said = builder.spoken(now: sections.now)
        #expect(said.contains("Builder \u{00B7} Sonnet"), "\(said)")
        #expect(said.contains("tile-grid"), "the lane it works for isn't said: \(said)")
        #expect(said.contains("380K tokens"), "\(said)")
        let loose = try #require(fleet.agents.first { $0.id == "agent:s1" }).spoken(now: sections.now)
        #expect(loose.contains("Design ov-503") && loose.contains("Not on a lane"), "\(loose)")
        #expect(loose.contains("Working"), "a subagent resumed for another round reads Working: \(loose)")
        #expect(!fleet.agents.map(\.id).contains("agent:r1"), "a finished agent is listed")
        // Working ones lead.
        let working = fleet.agents.map(\.working)
        #expect(working == working.sorted { $0 && !$1 })
    }

    @Test("no agents are drawn on a runner that doesn't list them")
    func agentsNeedTheCapability() throws {
        #expect(try F.sections(agents: false).shown.contains(.agents) == false)
    }

    @Test("themes are tiles: progress, cards and the one line for what waits on the owner; the finished ones are one tile")
    func themeTiles() throws {
        let grid = try F.sections().themes
        let first = try #require(grid.tiles.first { $0.name == "Teams and accounts" })
        #expect(first.spoken().contains("done"))
        let marketing = try #require(grid.tiles.first { $0.name == "Marketing" })
        #expect(marketing.spoken().contains("Needs you: How do we ship the site"))
        #expect(grid.finishedTitle == "Finished")
        #expect(grid.finished.map(\.name).contains("Mac look and feel"), "the finished tile names its themes")
        #expect(!grid.tiles.map(\.name).contains("Mac look and feel"), "a finished theme has a tile of its own")
        #expect(!(grid.tiles + grid.finished).map(\.name).contains("Abandoned"), "a dropped theme has none at all")
    }

    // MARK: Themes and Lanes

    @Test("Themes pushes each theme with its cards, and a theme's header opens its page")
    func themesList() throws {
        let plan = try F.plan("plan-phones-ov503.json")
        let outline = SidebarOutline.build(OneTreeInput(tasks: [], plan: plan))
        let theme = try #require(outline.themes.first { $0.title == "Teams and accounts" })
        #expect(theme.rowName == "Teams and accounts")
        #expect(theme.target == .theme(try #require(plan.themes.first { $0.name == "Teams and accounts" }).id), "its header opens its page")
        let all = outline.themes.flatMap { [$0] + $0.children }
        #expect(all.contains { $0.rowName == "ov-487" }, "no card under its theme")
    }

    @Test("Lanes groups them as the Plan does: Landing with its trains and lanes, and Waiting starts folded")
    func lanesList() throws {
        let plan = try F.plan("plan-phones-ov503.json")
        let sections = try F.sections()
        let outline = SidebarOutline.build(OneTreeInput(tasks: [], plan: plan))
        let landing = try #require(outline.lanes.first { $0.rowName == "Landing" })
        let train = try #require(landing.children.first { $0.rowName == "Train 86" })
        let lane = try #require((landing.children + train.children).first { $0.rowName == "team-dir" })
        #expect(!lane.detail.contains("Ready") && !lane.caption.contains("Ready"), "a lane aboard says nothing of its state: \(lane.detail)")
        #expect(lane.target == .lane(try #require(plan.lanes.first { $0.name == "team-dir" }).id), "a lane opens its page")
        let waiting = try #require(outline.lanes.first { $0.rowName == "Waiting" })
        #expect(waiting.kind == .group && !waiting.expandedByDefault, "Waiting starts folded")
        #expect(landing.expandedByDefault, "Landing starts open")
        let rows = waiting.children.map(\.rowName)
        #expect(rows.contains("hero-shot"), "\(rows)")
        #expect(sections.waiting.count > 0)
    }
}
