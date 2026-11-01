import Foundation
import Testing

@testable import AgentKit

/// What `PlanSectionsUITests` pinned by launching the app and scrolling, as
/// values over the same fixture (`plan-phones-ov503.json`, ov-538): the order of
/// the sections, the band's words, a landing and the words it speaks, Waiting's
/// groups, the fold, the agents, the themes and the two pushed lists.
struct PlanPhoneSectionsTests {
    typealias F = PlanPhoneFixtures

    // MARK: The band

    @Test("the band says Now and Next in two lines, and what needs the owner as its one mark")
    func theBand() throws {
        let sections = try F.sections()
        let band = PlanGlanceSentence(sections: sections)
        let spoken = band.spoken(needsYou: sections.needsYou)
        #expect(spoken.contains("Landing 86 checking on CI"), "\(spoken)")
        #expect(spoken.contains(" building") && spoken.contains(" in review"), "\(spoken)")
        #expect(spoken.contains("Next: A real window capture on the front page, then Onboarding"), "\(spoken)")
        #expect(spoken.hasSuffix(" needs you."), "\(spoken)")
    }

    @Test("at the sheet's medium detent the band says Now only; at large it says Next as well")
    func theBandAtMedium() throws {
        let sections = try F.sections()
        let band = PlanGlanceSentence(sections: sections)
        let medium = band.spoken(needsYou: 2, nowOnly: true)
        #expect(medium.contains("Landing 86 checking on CI"), "\(medium)")
        #expect(!medium.contains("Next:"), "the band says Next at the medium detent: \(medium)")
        #expect(medium.hasSuffix("2 need you."), "\(medium)")
        #expect(band.spoken(needsYou: 2, nowOnly: false).contains("Next:"))
        #expect(band.spoken(needsYou: 0, nowOnly: true) == "\(band.nowLine).", "nothing needs you, so nothing is added")
        #expect(band.spoken(needsYou: 1) .hasSuffix("1 needs you."))
    }

    // MARK: The sections

    @Test("the sections come in the owner's order: Landing, In Review, Building, Waiting, Waiting for the Next Landing, Agents, Themes")
    func order() throws {
        let sections = try F.sections()
        #expect(
            sections.shown == [.needsYou, .landing, .inReview, .building, .waiting, .agents, .themes],
            "\(sections.shown)")
        #expect(PlanSections.Kind.allCases == [.needsYou, .landing, .inReview, .building, .waiting, .onLandedLanding, .agents, .themes])
        #expect(PlanSections.Kind.allCases.map(\.title) == [
            "Needs You", "Landing", "In Review", "Building", "Waiting", "Waiting for the Next Landing", "Agents", "Themes",
        ])
    }

    @Test("a landing is Landing N with its title under it, where it stands on the ladder, and one waiting for main says so")
    func theLadderAndTheLandingWaitingForMain() throws {
        let sections = try F.sections()
        let now = sections.now
        func card(_ number: Int) throws -> PlanSections.LandingCard { try #require(sections.landings.first { $0.landing.number == number }) }
        let ready = try card(86).spoken(ci: nil, now: now)
        #expect(ready.hasPrefix("Landing 86, Teams on a shared runner"), "\(ready)")
        #expect(ready.contains("Checking on CI \u{00B7} step 2 of 3"), "\(ready)")
        #expect(ready.contains("3f78754f"), "the commit isn't said: \(ready)")
        #expect(ready.contains("Lander \u{00B7} opus"), "\(ready)")
        #expect(ready.contains("The team directory; the iPhone composer"), "\(ready)")
        #expect(!ready.contains("a-integ86"), "the lander's id is not in the text: \(ready)")

        let waiting = try card(89).spoken(ci: nil, now: now)
        #expect(waiting.contains("Waiting for Main's Checks to Pass \u{00B7} step 1 of 3"), "\(waiting)")
        #expect(!waiting.contains("Waiting for ios-overwrite"), "no ticket holds a landing: \(waiting)")

        #expect(try card(84).spoken(ci: nil, now: now).contains("Stalled \u{00B7} step 1 of 3"))
    }

    @Test("a ticket aboard a landing says nothing of its own state, and none says it holds the landing")
    func aTicketAboardSaysNothing() throws {
        let sections = try F.sections()
        let ready = try #require(sections.landings.first { $0.landing.number == 86 })
        let teamDir = try #require(ready.tickets.first { $0.key == "ov-487" })
        #expect(teamDir.note == nil, "a ticket aboard says nothing: ready for what?")
        let waiting = try #require(sections.landings.first { $0.landing.number == 89 })
        let holder = try #require(waiting.tickets.first { $0.key == "ov-485" })
        #expect(holder.note == nil && !(holder.standing.word.isEmpty), "its round isn't said aboard")
        #expect(holder.note?.contains("round 2") != true)
    }

    @Test("a long press explains what a landing is, then each step of its ladder, the one it stands on in the phase's words")
    func explanation() throws {
        let sections = try F.sections()
        let landing = try #require(sections.landings.first { $0.landing.number == 86 })
        let said = landing.explanation
        #expect(said.first == "Reviewed tickets land on main together, after one round of checks.")
        #expect(said.count == 1 + PlanLadder.steps.count)
        #expect(said.contains("Checking Here: The full test run, on the runner."), "\(said)")
        #expect(PlanWords.landingExplanation == said[0])
    }

    @Test("Waiting is ranked, then not ranked, and each group is named once something else shares the section")
    func waitingGroups() throws {
        let waiting = try F.sections().waiting
        #expect(waiting.groups.map(\.id) == ["plan-next-up", "plan-not-ranked", "plan-waiting-on-you"])
        #expect(waiting.groups.map(\.title) == ["Next Up", "Not Ranked", "Waiting on You"])
        #expect(waiting.namesGroups)
        let next = try #require(waiting.groups.first)
        #expect(next.rows.first?.line.key == "ov-302")
        #expect(next.rows.first?.line.reason == "Unblocked once the site is one project")
        #expect(next.rank(at: 0) == 1 && next.rank(at: 1) == 2, "the first ranked ticket is numbered 1")
        #expect(waiting.groups[1].rank(at: 0) == nil && waiting.groups[2].rank(at: 0) == nil)
        #expect(waiting.groups[2].attention && !next.attention)
        #expect(waiting.onYou.map(\.line.key) == ["ov-300"], "the ticket whose card needs a decision is Waiting on You")
        #expect(!waiting.next.map(\.line.key).contains("ov-300"))
    }

    @Test("a group alone in Waiting is not named")
    func aLoneGroupIsNotNamed() throws {
        let plan = try PlanModelTests.plan(
            themes: [], lines: [PlanModelTests.line("t1", "waiting", rank: 1)])
        let waiting = PlanSections(plan: plan).waiting
        #expect(waiting.groups.map(\.id) == ["plan-next-up"] && !waiting.namesGroups)
    }

    @Test("agents are who, for what and how many tokens, working ones first; one that finished is not listed")
    func agents() throws {
        let sections = try F.sections()
        let fleet = sections.fleet
        #expect(fleet.header(locale: F.en).contains("working"))
        let builder = try #require(fleet.agents.first { $0.id == "pane:b1" })
        let said = builder.spoken(now: sections.now)
        #expect(said.contains("Builder \u{00B7} sonnet"), "\(said)")
        #expect(said.contains("ov-310"), "the ticket it works for isn't said: \(said)")
        #expect(said.contains("380K tokens"), "\(said)")
        let loose = try #require(fleet.agents.first { $0.id == "agent:s1" }).spoken(now: sections.now)
        #expect(loose.contains("Design ov-503") && loose.contains(PlanWords.onNoTicket), "\(loose)")
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

    // MARK: Themes and Tickets

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

    @Test("Tickets groups them as the Plan does: Landing with its landings and tickets, and Waiting starts folded")
    func ticketsList() throws {
        let plan = try F.plan("plan-phones-ov503.json")
        let sections = try F.sections()
        let outline = SidebarOutline.build(OneTreeInput(tasks: [], plan: plan))
        let landing = try #require(outline.tickets.first { $0.rowName == "Landing" })
        let landingRow = try #require(landing.children.first { $0.rowName == "Landing 86" })
        let ticket = try #require((landing.children + landingRow.children).first { $0.rowName == "ov-487" })
        #expect(!ticket.detail.contains("Ready") && !ticket.caption.contains("Ready"), "a ticket aboard says nothing of its state: \(ticket.detail)")
        #expect(ticket.target == .ticket(try #require(plan.line(key: "ov-487")).task), "a ticket opens its page")
        let waiting = try #require(outline.tickets.first { $0.rowName == "Waiting" })
        #expect(waiting.kind == .group && !waiting.expandedByDefault, "Waiting starts folded")
        #expect(landing.expandedByDefault, "Landing starts open")
        let rows = waiting.children.map(\.rowName)
        #expect(rows.contains("ov-302"), "\(rows)")
        #expect(sections.waiting.count > 0)
    }
}
