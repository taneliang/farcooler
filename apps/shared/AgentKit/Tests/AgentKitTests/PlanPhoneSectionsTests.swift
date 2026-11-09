import Foundation
import Testing

@testable import AgentKit

/// What `PlanSectionsUITests` pinned by launching the app and scrolling, as
/// values over the same fixture (`plan-phones-ov503.json`, ov-538): the band's
/// words, a landing and the words it speaks, Waiting's groups, the themes and
/// the two pushed lists. What is in each section, and its words, is the Rust
/// core's view (ov-1016); these check what the phone says from it, and that
/// what the phone knows beside the plan (the board's statuses, whether the
/// runner lists agents) reaches it.
struct PlanPhoneSectionsTests {
    typealias F = PlanPhoneFixtures

    // MARK: The band

    @Test("the band says Now and Next in two lines, and what needs the owner as its one mark")
    func theBand() throws {
        let view = try F.view()
        let band = PlanGlanceSentence(view: view)
        let spoken = band.spoken(needsYou: Int(view.needsYou))
        #expect(spoken.contains("Landing 86 checking on CI"), "\(spoken)")
        #expect(spoken.contains(" building") && spoken.contains(" in review"), "\(spoken)")
        #expect(spoken.contains("Next: A real window capture on the front page, then Onboarding"), "\(spoken)")
        #expect(spoken.hasSuffix(" needs you."), "\(spoken)")
    }

    @Test("at the sheet's medium detent the band says Now only; at large it says Next as well")
    func theBandAtMedium() throws {
        let band = PlanGlanceSentence(view: try F.view())
        let medium = band.spoken(needsYou: 2, nowOnly: true)
        #expect(medium.contains("Landing 86 checking on CI"), "\(medium)")
        #expect(!medium.contains("Next:"), "the band says Next at the medium detent: \(medium)")
        #expect(medium.hasSuffix("2 need you."), "\(medium)")
        #expect(band.spoken(needsYou: 2, nowOnly: false).contains("Next:"))
        #expect(band.spoken(needsYou: 0, nowOnly: true) == "\(band.nowLine).", "nothing needs you, so nothing is added")
        #expect(band.spoken(needsYou: 1) .hasSuffix("1 needs you."))
    }

    // MARK: The sections

    @Test("a landing is Landing N with its title under it, where it stands on the ladder, and one waiting for main says so")
    func theLadderAndTheLandingWaitingForMain() throws {
        let view = try F.view()
        let now = view.now
        func card(_ number: Int) throws -> WireLandingCard { try #require(view.landings.first { $0.number == number }) }
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

    @Test("landing speech preserves the recorded model identity without repeating literal IDs")
    func landingModelIdentity() throws {
        // The lander's words are the view's ("Lander · GPT-6.1 Sol"); the raw
        // model it carries is what the screen reader adds.
        var card = try #require(F.view().landings.first { $0.number == 86 })
        card.landerWords = "Lander \u{00B7} GPT-6.1 Sol"
        card.lander?.model = "gpt-6.1-sol"
        #expect(card.facts(ci: nil, now: 0).contains("GPT-6.1 Sol"))
        #expect(card.spoken(ci: nil, now: 0).contains("Model ID: gpt-6.1-sol"))
        card.landerWords = "Lander \u{00B7} gpt-6.1-sol-20260101"
        card.lander?.model = "gpt-6.1-sol-20260101"
        let literal = card.spoken(ci: nil, now: 0)
        #expect(literal.components(separatedBy: "gpt-6.1-sol-20260101").count == 2)
        card.lander?.model = ""
        #expect(!card.spoken(ci: nil, now: 0).contains("Model ID:"))
        card.lander = nil
        card.landerWords = nil
        #expect(!card.spoken(ci: nil, now: 0).contains("Model ID:"))
    }

    @Test("a long press explains what a landing is, then each step of its ladder, the one it stands on in the phase's words")
    func explanation() throws {
        let view = try F.view()
        let landing = try #require(view.landings.first { $0.number == 86 })
        let said = landing.explanation(landing: view.section(.landing)?.help)
        #expect(said.first == "Reviewed tickets land on main together, after one round of checks.")
        #expect(said.count == 1 + landing.ladder.steps.count)
        #expect(said.contains("Checking Here: The full test run, on the runner."), "\(said)")
    }

    @Test("Waiting is ranked, then not ranked, then on you, each group named, and the board's decision reaches it")
    func waitingGroups() throws {
        let view = try F.view()
        #expect(view.waiting.map(\.id) == ["plan-next-up", "plan-not-ranked", "plan-waiting-on-you"])
        #expect(view.waiting.map(\.title) == ["Next Up", "Not Ranked", "Waiting on You"])
        #expect(view.namesWaitingGroups)
        #expect(view.waiting(.next).first?.key == "ov-302" && view.waiting(.next).first?.rank == 1)
        #expect(view.waiting.map(\.attention) == [false, false, true])
        #expect(view.waiting(.onYou).map(\.key) == ["ov-300"], "the ticket whose card needs a decision is Waiting on You")
    }

    @Test("no agents are drawn on a runner that doesn't list them")
    func agentsNeedTheCapability() throws {
        #expect(try F.view(agents: false).section(.agents) == nil)
        #expect(try F.view(agents: true).section(.agents) != nil)
    }

    @Test("themes are tiles: progress, cards and the one line for what waits on the owner; the finished ones are one tile")
    func themeTiles() throws {
        let grid = try F.view().themes
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
        let view = try F.view()
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
        #expect(view.waitingCount > 0)
    }
}
