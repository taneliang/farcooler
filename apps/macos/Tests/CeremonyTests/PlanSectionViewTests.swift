import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Plan's sections on the Mac (ov-503, phase 5): each section drawn, what
/// it says empty, the two columns from 960 pt and the one column below, no
/// 680-pt cap, and engine landing groups. The
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

    @Test("Engine landing groups wrap long titles at phone width")
    func landingQueueAtNarrowWidth() async throws {
        let store = try await Self.store(.busy)
        var view = try Self.view(store)
        let title = String(repeating: "A long ticket title that must wrap completely. ", count: 5)
        view.landingGroups = [WireLandingGroupView(id: "test-group", title: "Landing group 12", lines: [
            "Run gate · Running", "4 min in this step", "ov-1105 · \(title)",
            "Split after a gate failed", "Failed job: cargo test", "error: expected a passing gate",
        ])]
        let landingIndex = try #require(view.sections.firstIndex { $0.kind == .landing })
        view.sections[landingIndex].count = 1
        view.sections[landingIndex].empty = nil
        let frames = await PlanThemePageTests.frames(PlanSectionsView(
            view: view, model: store.plan.plan, statuses: [:], plan: store.plan,
            selected: nil, keyed: false, defaults: PlanViewTests.defaults(), onOpen: { _ in }), width: 320, height: 3200)
        let group = try #require(frames["plan-landing-group-test-group"])
        #expect(group.minX >= 0 && group.maxX <= 320.5)
        view.landingGroups?[0].lines[2] = "ov-1105 · Short title"
        let short = await PlanThemePageTests.frames(PlanSectionsView(
            view: view, model: store.plan.plan, statuses: [:], plan: store.plan,
            selected: nil, keyed: false, defaults: PlanViewTests.defaults(), onOpen: { _ in }), width: 320, height: 3200)
        let shortGroup = try #require(short["plan-landing-group-test-group"])
        #expect(group.height > shortGroup.height + 40, "the complete title wraps vertically")
    }

    @Test("Landing, In Review, Building, Waiting, Agents and Themes are drawn in the owner's order")
    func sectionsInOrder() async throws {
        let drawn = try await Self.drawn(.busy, width: 600)
        let order = ["plan-landing", "plan-in-review", "plan-building", "plan-waiting", "plan-agents", "plan-themes"]
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

    @Test("Numbered landings from an older runner do not appear in the Plan")
    func oldLandingsAreHidden() async throws {
        let store = try await Self.store(.busy)
        #expect(store.plan.plan.landings.contains { $0.number == 86 })
        let view = try Self.view(store)
        #expect(view.landings.isEmpty)
        let drawn = try await Self.drawn(.busy, width: 600)
        #expect(drawn["plan-landing-86"] == nil)
        #expect(drawn["plan-landing-empty"] != nil)
    }

    @Test("Each section says what it says empty, and Needs You-free quiet boards draw nothing else")
    func emptyStates() async throws {
        let quiet = try await Self.drawn(.quiet, width: 600)
        for id in ["plan-landing-empty", "plan-waiting-empty"] { #expect(quiet[id] != nil, "\(id) in \(quiet.keys.sorted())") }
        #expect(quiet["plan-in-review"] == nil && quiet["plan-building"] == nil, "In Review and Building say nothing with nothing")
        let none = try await Self.drawn(.emptyNext, width: 600)
        #expect(none["plan-landing-empty"] != nil)
        #expect(none["plan-next-up"] == nil && none["plan-not-ranked"] != nil)
        #expect(none["plan-ticket-ov-474"] != nil)
        #expect(none["plan-agents"] != nil && none["plan-agents-empty"] == nil)
        let store = try await Self.store(.quiet)
        let view = try Self.view(store)
        #expect(view.section(.landing)?.empty == "Nothing is landing right now." && view.section(.agents)?.empty == "No agents running.")
        let band = PlanGlanceSentence(view: view)
        #expect(band.text == "No tickets moving. Next: nothing queued.")
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
        #expect(PlanGlanceSentence(view: view).now?.contains("Landing 88") != true)
    }

}
