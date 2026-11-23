import Foundation
import Testing

@testable import AgentKit

/// What `PlanSectionsUITests` pinned by launching the app and scrolling, as
/// values over the same fixture (`plan-phones-ov503.json`, ov-538): the band's
/// words, historical landing absence, Waiting's groups, the themes and
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
        #expect(!spoken.contains("Landing 86"), "\(spoken)")
        #expect(spoken.contains(" building") && spoken.contains(" in review"), "\(spoken)")
        #expect(spoken.contains("Next: A real window capture on the front page, then Onboarding"), "\(spoken)")
        #expect(spoken.hasSuffix(" needs you."), "\(spoken)")
    }

    @Test("at the sheet's medium detent the band says Now only; at large it says Next as well")
    func theBandAtMedium() throws {
        let band = PlanGlanceSentence(view: try F.view())
        let medium = band.spoken(needsYou: 2, nowOnly: true)
        #expect(!medium.contains("Landing 86"), "\(medium)")
        #expect(!medium.contains("Next:"), "the band says Next at the medium detent: \(medium)")
        #expect(medium.hasSuffix("2 need you."), "\(medium)")
        #expect(band.spoken(needsYou: 2, nowOnly: false).contains("Next:"))
        #expect(band.spoken(needsYou: 0, nowOnly: true) == "\(band.nowLine).", "nothing needs you, so nothing is added")
        #expect(band.spoken(needsYou: 1) .hasSuffix("1 needs you."))
    }

    // MARK: The sections

    @Test("historical landing rows are absent from the phone sections")
    func historicalLandingsAreAbsent() throws {
        let plan = try F.plan("plan-phones-ov503.json")
        #expect(!plan.landings.isEmpty)
        #expect(try F.view().landings.isEmpty)
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

    @Test("Tickets omit historical landing rows and Waiting starts folded")
    func ticketsList() throws {
        let plan = try F.plan("plan-phones-ov503.json")
        let view = try F.view()
        let outline = SidebarOutline.build(OneTreeInput(tasks: [], plan: plan))
        #expect(!outline.tickets.flatMap(\.children).contains { $0.rowName == "Landing 86" })
        let waiting = try #require(outline.tickets.first { $0.rowName == "Waiting" })
        #expect(waiting.kind == .group && !waiting.expandedByDefault, "Waiting starts folded")
        let rows = waiting.children.map(\.rowName)
        #expect(rows.contains("ov-302"), "\(rows)")
        #expect(view.waitingCount > 0)
    }
}
