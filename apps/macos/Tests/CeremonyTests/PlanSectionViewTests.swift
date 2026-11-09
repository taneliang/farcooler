import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Plan's sections on the Mac (ov-503, phase 5): each section drawn, what
/// it says empty, the two columns from 960 pt and the one column below, no
/// 680-pt cap, a landing's tickets inside its card, and the folded stranded section. The
/// rules and the words are the Rust core's view (`WirePlanView`, ov-1016), pinned
/// by its tests; this holds the real views to it over `PlanScenario`'s plans,
/// through the bridge the app calls.
@MainActor
@Suite(.serialized)
struct PlanSectionViewTests {
    static func store(_ kind: PlanScenario.Kind) async throws -> TaskBoardStore {
        let calls = PlanViewTests.Calls()
        calls.plan = try PlanScenario.data(kind)
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        return store
    }

    /// The Rust core's view of the store's plan.
    static func view(_ store: TaskBoardStore, needsYou: Int = 0, statuses: [String: TaskStatus] = [:]) throws -> WirePlanView {
        try #require(store.plan.view(needsYou: needsYou, statuses: statuses, activity: [:]), "the bridge built no view")
    }

    /// What the overview drew in a canvas `width` wide.
    static func drawn(_ kind: PlanScenario.Kind, width: CGFloat, defaults: UserDefaults? = nil) async throws -> [String: CGRect] {
        let store = try await Self.store(kind)
        let defaults = defaults ?? PlanViewTests.defaults()
        return await PlanThemePageTests.frames(
            PlanOverviewView(
                plan: store.plan, statuses: store.board.statuses, selected: nil, needsYou: 1, onOpen: { _ in }, defaults: defaults),
            width: width, height: 3200)
    }

    @Test("Landing, In Review, Building, Waiting, Agents and Themes are drawn in the owner's order")
    func sectionsInOrder() async throws {
        let drawn = try await Self.drawn(.busy, width: 600)
        let order = ["plan-landing", "plan-in-review", "plan-building", "plan-waiting", "plan-on-landed-landing", "plan-agents", "plan-themes"]
        let frames = try order.map { try #require(drawn[$0], "\($0) in \(drawn.keys.sorted())") }
        for (a, b) in zip(frames, frames.dropFirst()) { #expect(a.maxY <= b.minY + 0.5, "\(a) then \(b)") }
    }

    @Test("From 960 pt the work in motion is the left column and what is queued the right; below, one column")
    func twoColumns() async throws {
        let wide = try await Self.drawn(.busy, width: 900)
        let landing = try #require(wide["plan-landing"])
        let waiting = try #require(wide["plan-waiting"])
        let agents = try #require(wide["plan-agents"])
        #expect(waiting.minX > landing.maxX, "Waiting is beside Landing: \(landing) \(waiting)")
        #expect(abs(waiting.minY - landing.minY) < 1, "the columns start together")
        #expect(abs(agents.minX - waiting.minX) < 1 && agents.minY >= waiting.maxY, "Agents is under Waiting")
        let themes = try #require(wide["plan-themes"])
        #expect(themes.minY >= max(landing.maxY, agents.maxY) - 0.5 && themes.width > landing.width * 1.8, "Themes spans both")
        let narrow = try await Self.drawn(.busy, width: 700)
        let stacked = try #require(narrow["plan-waiting"])
        #expect(stacked.minX == narrow["plan-landing"]?.minX && stacked.minY > (narrow["plan-building"]?.maxY ?? .infinity) - 0.5, "stacked")
    }

    @Test("There's no 680-pt cap: the sections take a 1,400-pt canvas")
    func noReadingWidth() async throws {
        let drawn = try await Self.drawn(.busy, width: 1400)
        let sections = try #require(drawn["plan-sections"])
        #expect(sections.width > 1300, "\(sections.width)")
    }

    @Test("Landing 86 draws its two tickets in its card, and names the phase it stands in")
    func landingCarriesItsTickets() async throws {
        let store = try await Self.store(.busy)
        let landing = try #require(store.plan.plan.landings.first { $0.number == 86 })
        #expect(landing.tickets.map(\.key) == ["ov-487", "ov-492"], "the landing lists its tickets")
        let drawn = try await Self.drawn(.busy, width: 600)
        #expect(drawn["plan-landing-86"] != nil, "the landing is drawn")
        let card = try #require(drawn["plan-landing-86"])
        let inside = ["plan-landing-ticket-ov-487", "plan-landing-ticket-ov-492"].compactMap { drawn[$0] }
        #expect(inside.count == 2 && inside.allSatisfy { card.contains($0) }, "both tickets are inside the card: \(drawn.keys.sorted())")
        #expect(drawn["plan-ticket-ov-487"] == nil && drawn["plan-ticket-ov-492"] == nil, "and not drawn again as rows")
        let view = try Self.view(store)
        let aboard = try #require(view.landings.first { $0.number == 86 })
        let spoken = aboard.spoken(ci: nil, now: view.now)
        #expect(spoken.hasPrefix("Landing 86, Teams on a shared runner, "), "\(spoken)")
        #expect(spoken.contains("Waiting for Tickets \u{00B7} step 1 of 3"), "it says where it stands: \(spoken)")
    }

    @Test("Waiting for Main's Checks to Pass is a landing's phase, drawn on the card and said in its label")
    func waitingForMain() async throws {
        let store = try await Self.store(.ladder)
        let card = try #require(try Self.view(store).landings.first { $0.number == 94 })
        #expect(card.phase.rawValue == "waiting_for_main")
        #expect(card.phaseWord == "Waiting for Main's Checks to Pass")
        let drawn = try await Self.drawn(.ladder, width: 600)
        #expect(drawn["plan-landing-94"] != nil && drawn["plan-ladder-step-0"] != nil, "\(drawn.keys.sorted())")
        let widths = await PlanThemePageTests.frames(Self.ladder(card), width: 560, height: 120)
        #expect((0..<3).allSatisfy { widths["plan-ladder-step-\($0)"] != nil }, "\(widths.keys.sorted())")
    }

    @Test("A landing whose CI failed reads its phase, commit and jobs; the ladder stands on its step")
    func ciFailedLanding() async throws {
        let store = try await Self.store(.busy)
        let view = try Self.view(store)
        let card = try #require(view.landings.first { $0.number == 88 })
        let landing = try #require(store.plan.plan.landings.first { $0.id == card.id })
        let words = PlanLandingCard.facts(card, ci: store.plan.plan.ci(of: landing), now: view.now)
        #expect(words.hasPrefix("3f78754f \u{00B7} Failed \u{00B7} 1 of 3 jobs failed"), "\(words)")
        #expect(card.phase == .ciFailed && card.attention)
        #expect(card.ladder.at == 1 && card.ladder.steps.map(\.label) == ["Checking Here", "CI Failed", "Landed"])
        #expect(view.landings.first?.number == 88, "furthest along first")
    }

    static func ladder(_ card: WireLandingCard) -> PlanLadderView {
        PlanLadderView(ladder: card.ladder, word: card.phaseWord, explanation: card.explanation, attention: card.attention)
    }

    @Test("Each section says what it says empty, and Needs You-free quiet boards draw nothing else")
    func emptyStates() async throws {
        let quiet = try await Self.drawn(.quiet, width: 600)
        for id in ["plan-landing-empty", "plan-waiting-empty"] { #expect(quiet[id] != nil, "\(id) in \(quiet.keys.sorted())") }
        #expect(quiet["plan-in-review"] == nil && quiet["plan-building"] == nil, "In Review and Building say nothing with nothing")
        let none = try await Self.drawn(.emptyNext, width: 600)
        #expect(none["plan-waiting-empty"] != nil && none["plan-landing-empty"] == nil, "Next is empty; landings are landing")
        #expect(none["plan-agents"] != nil && none["plan-agents-empty"] == nil)
        let store = try await Self.store(.quiet)
        let view = try Self.view(store)
        #expect(view.section(.landing)?.empty == "Nothing is landing right now." && view.section(.agents)?.empty == "No agents running.")
        let band = PlanGlanceSentence(view: view)
        #expect(band.text == "No tickets moving. Next: nothing queued.")
    }

    @Test("Tickets stranded on a landed landing are in a section folded until opened")
    func strandedFold() async throws {
        let defaults = PlanViewTests.defaults()
        let closed = try await Self.drawn(.busy, width: 600, defaults: defaults)
        #expect(closed["plan-on-landed-landing"] != nil && closed["plan-ticket-ov-474"] == nil, "folded: rows aren't drawn")
        let store = try await Self.store(.busy)
        #expect(Set(try Self.view(store).onLandedLanding.map(\.key)) == ["ov-474", "ov-482"])
        let key = "board.plan.section.plan-on-landed-landing.\(store.plan.host).\(store.plan.workspace.id)"
        defaults.set(true, forKey: key)
        let open = try await Self.drawn(.busy, width: 600, defaults: defaults)
        #expect(open["plan-ticket-ov-474"] != nil && open["plan-ticket-ov-482"] != nil, "opened: \(open.keys.sorted())")
    }

    @Test("Tickets in review and building say their agent and tokens; Next Up is ranked, with a group each for the rest")
    func ticketRows() async throws {
        let store = try await Self.store(.busy)
        let view = try Self.view(store, statuses: store.board.statuses)
        let building = try #require(view.building.first { $0.key == "ov-490" })
        #expect(building.second == "Building \u{00B7} ov-490 \u{00B7} Builder \u{00B7} sonnet \u{00B7} 380K \u{00B7} 1 h", "\(building.second)")
        #expect(view.waiting(.next).map(\.key) == ["ov-494", "ov-495", "ov-496"])
        #expect(view.waiting(.onYou).map(\.key) == ["ov-1"], "a card waiting on a decision")
        let drawn = try await Self.drawn(.busy, width: 600)
        for id in ["plan-next-up", "plan-not-ranked", "plan-waiting-on-you"] { #expect(drawn[id] != nil, "\(id)") }
        // The row draws the view's words: its title, theme, second line and notes.
        let row = await PlanThemePageTests.frames(
            PlanCardRow(card: building, selected: false, keyed: false) {}, width: 560, height: 200)
        #expect(row["plan-ticket-ov-490"] != nil, "\(row.keys.sorted())")
        if building.theme != nil { #expect(row["plan-ticket-ov-490-theme"] != nil, "\(row.keys.sorted())") }
    }

    @Test("Agents: a header, one row per agent that exists, working first with tokens then dollars in columns, and no Finished fold")
    func agents() async throws {
        let store = try await Self.store(.busy)
        let agents = try Self.view(store).agents
        #expect(agents.rows.map(\.id).first == "pane:orchestrator")
        #expect(agents.rows.map(\.id).last == "pane:idle", "the idle pane that still runs is last")
        #expect(agents.header.hasPrefix("\(agents.rows.count) agents, including the orchestrator \u{00B7} 5 working \u{00B7} 6.3M tokens in the last 24 hours"), "the day's total counts the finished reviewer")
        let drawn = try await Self.drawn(.busy, width: 600)
        #expect(drawn["plan-agents-header"] != nil)
        #expect(drawn["plan-agents-finished"] == nil, "there is no Finished fold")
        let orchestrator = try #require(drawn["plan-agent-pane:orchestrator"])
        let builder = try #require(drawn["plan-agent-agent:b1"])
        #expect(abs(orchestrator.maxX - builder.maxX) < 0.5, "the columns share a right edge")
        #expect(drawn["plan-agent-agent:r1"] == nil, "a finished agent is not listed")
        let subagent = try #require(agents.rows.first { $0.second.hasPrefix("\u{201C}Design ov-503\u{201D}") })
        #expect(subagent.working, "a subagent resumed for another round reads Working")
        #expect(drawn["plan-agent-\(subagent.id)"] != nil && drawn["plan-agent-pane:idle"] != nil)
        #expect(subagent.second.hasPrefix("\u{201C}Design ov-503\u{201D} \u{00B7} Not on a ticket \u{00B7} Working"))
    }

    @Test("Themes: a tile each with its progress, cards and ask, finished ones in one tile, over-budget in amber")
    func themes() async throws {
        let drawn = try await Self.drawn(.busy, width: 960)
        for name in ["Teams and accounts", "Phones", "Mac look and feel"] { #expect(drawn["plan-theme-tile-\(name)"] != nil, "\(name)") }
        #expect(drawn["plan-theme-finished"] != nil && drawn["plan-tile-Teams and accounts-over-budget"] != nil)
        #expect(drawn["plan-tile-Phones-over-budget"] == nil)
        let tiles = ["Teams and accounts", "Phones", "Mac look and feel"].compactMap { drawn["plan-theme-tile-\($0)"] }
        #expect(tiles.count == 3 && tiles.allSatisfy { $0.width >= PlanMetrics.tile - 0.5 }, "no tile narrower than \(PlanMetrics.tile)")
        let narrow = try await Self.drawn(.busy, width: 300)
        #expect(try #require(narrow["plan-theme-tile-Phones"]).width > 240, "one column on a narrow canvas")
    }

    @Test("The Plan's home draws no pinned Now and Next band: the column carries the same, and Needs You is not said twice")
    func noBand() async throws {
        let store = try await Self.store(.busy)
        let drawn = await PlanThemePageTests.frames(
            PlanHome(board: store, needsYou: PlanNeedsYou(), onOpen: { _ in }), width: 700, height: 3200)
        #expect(drawn["plan-home"] != nil && drawn["plan-pipeline"] != nil, "\(drawn.keys.sorted())")
        for id in ["plan-band-now", "plan-band-next", "plan-band-needs-you"] { #expect(drawn[id] == nil, "\(id) is gone") }
        // The sentence is still the whole summary where there is no room for sections.
        let view = try Self.view(store, needsYou: 2, statuses: store.board.statuses)
        #expect(PlanGlanceSentence(view: view).now?.hasPrefix("Landing 88 CI failed") == true)
    }

    @Test("A landing is Landing N with its title under it, and its ladder explains each step")
    func landingWords() async throws {
        let drawn = try await Self.drawn(.busy, width: 600)
        #expect(drawn["plan-landing-86-label"] != nil && drawn["plan-landing-86-subtitle"] != nil, "Landing 86, then its title")
        #expect(drawn["plan-landing-88-subtitle"] == nil, "a title that says no more than the label is not said again")
        #expect(drawn["plan-landing-ticket-ov-487-note"] == nil, "a ticket simply aboard says nothing")
        let view = try Self.view(try await Self.store(.busy))
        let titles = view.sections.map(\.title) + view.landings.flatMap { $0.ladder.steps.map(\.label) }
        #expect(titles.allSatisfy { !$0.contains("Shipping") && !$0.contains("Picked") && !$0.contains("Gated") })
        #expect(view.section(.landing)?.help == "Reviewed tickets land on main together, after one round of checks.")
    }

    @Test("With no agent alive the Agents section still says what the day spent, and the Landing ladders line up card to card")
    func ladderAndSpend() async throws {
        let store = try await Self.store(.ladder)
        let view = try Self.view(store)
        let said = "No agents running \u{00B7} 1.2M tokens in the last 24 hours \u{00B7} \u{2248}$12.00"
        #expect(view.section(.agents)?.empty == said, "\(view.section(.agents)?.empty ?? "-")")
        let drawn = try await Self.drawn(.ladder, width: 600)
        #expect(drawn["plan-agents-empty"] != nil)
        // (A point of rounding at most.) The first step is the same width whatever it says: a long phase wraps in its own column.
        var widths: [[CGFloat]] = []
        #expect(view.landings.count >= 2, "the ladder scenario's landings: \(view.landings.map(\.phase))")
        for card in view.landings {
            let frames = await PlanThemePageTests.frames(Self.ladder(card), width: 560, height: 120)
            widths.append((0..<3).compactMap { frames["plan-ladder-step-\($0)"]?.width })
        }
        #expect(widths.allSatisfy { $0.count == 3 }, "\(widths)")
        for row in widths { #expect(row.allSatisfy { abs($0 - row[0]) <= 1 }, "the steps of one ladder are equal: \(row)") }
        #expect(abs(widths[0][0] - widths[1][0]) <= 1, "and equal from card to card: \(widths)")
    }
}
