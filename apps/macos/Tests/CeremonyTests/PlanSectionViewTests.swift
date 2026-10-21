import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Plan's sections on the Mac (ov-503, phase 5): each section drawn, what
/// it says empty, the two columns from 960 pt and the one column below, no
/// 680-pt cap, an old runner's held_by drawing nothing, and the folded stranded section. The
/// rules are AgentKit's (`PlanSectionsTests`); this holds the real views to
/// them over `PlanScenario`'s plans.
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
        let order = ["plan-landing", "plan-in-review", "plan-building", "plan-waiting", "plan-on-landed-train", "plan-agents", "plan-themes"]
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

    @Test("A runner from before Train 119 still sends held_by on train 86: no held line, no lane button, and no lane in amber")
    func oldRunnerHeldBy() async throws {
        let store = try await Self.store(.busy)
        let train = try #require(store.plan.plan.trains.first { $0.name == "train-86" })
        #expect(train.lanes.map(\.name) == ["team-dir", "ios-overwrite"], "the train still reads")
        let drawn = try await Self.drawn(.busy, width: 600)
        #expect(drawn["plan-train-train-86"] != nil, "the train is drawn")
        #expect(drawn.keys.allSatisfy { !$0.hasSuffix("-held") && !$0.hasSuffix("-move") }, "\(drawn.keys.sorted())")
        let sections = store.plan.sections(needsYou: 0, statuses: [:], activity: [:])
        let card = try #require(sections.trains.first { $0.train.name == "train-86" })
        #expect(card.spoken(ci: nil, now: sections.now).contains("Waiting for ios-overwrite") == false)
    }

    @Test("Waiting for Main's Checks to Pass is a train's phase, drawn on the card and said in its label")
    func waitingForMain() async throws {
        let store = try await Self.store(.ladder)
        let sections = store.plan.sections(needsYou: 0, statuses: [:], activity: [:])
        let card = try #require(sections.trains.first { $0.train.name == "train-94" })
        #expect(card.phase == .waitingForMain)
        #expect(card.phase.word == "Waiting for Main's Checks to Pass")
        let drawn = try await Self.drawn(.ladder, width: 600)
        #expect(drawn["plan-train-train-94"] != nil && drawn["plan-ladder-step-0"] != nil, "\(drawn.keys.sorted())")
        let widths = await PlanThemePageTests.frames(PlanLadderView(phase: .waitingForMain), width: 560, height: 120)
        #expect((0..<3).allSatisfy { widths["plan-ladder-step-\($0)"] != nil }, "\(widths.keys.sorted())")
    }

    @Test("A train whose CI failed reads its phase, commit and jobs; the ladder stands on its step")
    func ciFailedTrain() async throws {
        let store = try await Self.store(.busy)
        let sections = store.plan.sections(needsYou: 0, statuses: [:], activity: [:])
        let card = try #require(sections.trains.first { $0.train.name == "train-88" })
        let words = PlanTrainCard.facts(card, ci: store.plan.plan.ci(of: card.train), now: sections.now)
        #expect(words.hasPrefix("3f78754f \u{00B7} Failed \u{00B7} 1 of 3 jobs failed"), "\(words)")
        #expect(card.phase == .ciFailed && card.phase.needsAttention)
        #expect(PlanLadder.step(.ciFailed) == 1 && PlanLadder.step(.waitingForCi) == 1 && PlanLadder.step(.landed) == 3)
        #expect(PlanLadder.label(1, current: 1, phase: .ciFailed) == "CI Failed" && PlanLadder.label(0, current: 1, phase: .ciFailed) == "Checking Here")
        #expect(sections.trains.first?.train.name == "train-88", "furthest along first")
    }

    @Test("Each section says what it says empty, and Needs You-free quiet boards draw nothing else")
    func emptyStates() async throws {
        let quiet = try await Self.drawn(.quiet, width: 600)
        for id in ["plan-landing-empty", "plan-waiting-empty"] { #expect(quiet[id] != nil, "\(id) in \(quiet.keys.sorted())") }
        #expect(quiet["plan-in-review"] == nil && quiet["plan-building"] == nil, "In Review and Building say nothing with nothing")
        let none = try await Self.drawn(.emptyNext, width: 600)
        #expect(none["plan-waiting-empty"] != nil && none["plan-landing-empty"] == nil, "Next is empty; trains are landing")
        #expect(none["plan-agents"] != nil && none["plan-agents-empty"] == nil)
        let store = try await Self.store(.quiet)
        let sections = store.plan.sections(needsYou: 0, statuses: [:], activity: [:])
        #expect(sections.emptyWords(.landing) == "Nothing is landing right now." && sections.emptyWords(.agents) == "No agents running.")
        let band = PlanGlanceSentence(sections: sections)
        #expect(band.text == "No lanes moving. Next: nothing queued.")
    }

    @Test("Lanes stranded on a landed train are in a section folded until opened")
    func strandedFold() async throws {
        let defaults = PlanViewTests.defaults()
        let closed = try await Self.drawn(.busy, width: 600, defaults: defaults)
        #expect(closed["plan-on-landed-train"] != nil && closed["plan-lane-old-one"] == nil, "folded: rows aren't drawn")
        let store = try await Self.store(.busy)
        let sections = store.plan.sections(needsYou: 0, statuses: [:], activity: [:])
        #expect(Set(sections.onLandedTrain.map(\.lane.name)) == ["old-one", "old-two"])
        let key = "board.plan.section.plan-on-landed-train.\(store.plan.host).\(store.plan.workspace.id)"
        defaults.set(true, forKey: key)
        let open = try await Self.drawn(.busy, width: 600, defaults: defaults)
        #expect(open["plan-lane-old-one"] != nil && open["plan-lane-old-two"] != nil, "opened: \(open.keys.sorted())")
    }

    @Test("Lanes in review and building say their agent and tokens; Next Up is ranked, with a group each for the rest")
    func laneRows() async throws {
        let store = try await Self.store(.busy)
        let sections = store.plan.sections(needsYou: 0, statuses: store.board.statuses, activity: [:])
        let building = try #require(sections.building.first { $0.lane.name == "site-ship" })
        let words = PlanLaneRow.second(building.lane, agent: building.agent, landedTrain: nil, now: sections.now)
        #expect(words == "Building \u{00B7} ov-490 \u{00B7} Builder \u{00B7} Sonnet \u{00B7} 380K \u{00B7} 1 h", "\(words)")
        #expect(sections.waiting.next.map(\.lane.name) == ["reps", "skill-init", "watch-glance"])
        #expect(sections.waiting.onYou.map(\.lane.name) == ["asks-you"], "a card waiting on a decision")
        let drawn = try await Self.drawn(.busy, width: 600)
        for id in ["plan-next-up", "plan-not-ranked", "plan-waiting-on-you"] { #expect(drawn[id] != nil, "\(id)") }
    }

    @Test("Agents: a header, one row per agent that exists, working first with tokens then dollars in columns, and no Finished fold")
    func agents() async throws {
        let store = try await Self.store(.busy)
        let fleet = store.plan.sections(needsYou: 0, statuses: [:], activity: [:]).fleet
        #expect(fleet.agents.map(\.id).first == "pane:orchestrator")
        #expect(fleet.agents.map(\.id).last == "pane:idle", "the idle pane that still runs is last")
        #expect(fleet.header(locale: Locale(identifier: "en_US")).hasPrefix("\(fleet.agents.count) agents, including the orchestrator \u{00B7} 5 working \u{00B7} 6.3M tokens in the last 24 hours"), "the day's total counts the finished reviewer")
        let drawn = try await Self.drawn(.busy, width: 600)
        #expect(drawn["plan-agents-header"] != nil)
        #expect(drawn["plan-agents-finished"] == nil, "there is no Finished fold")
        let orchestrator = try #require(drawn["plan-agent-pane:orchestrator"])
        let builder = try #require(drawn["plan-agent-agent:b1"])
        #expect(abs(orchestrator.maxX - builder.maxX) < 0.5, "the columns share a right edge")
        #expect(drawn["plan-agent-agent:r1"] == nil, "a finished agent is not listed")
        let subagent = try #require(fleet.agents.first { $0.role == .subagent })
        #expect(subagent.working, "a subagent resumed for another round reads Working")
        #expect(drawn["plan-agent-\(subagent.id)"] != nil && drawn["plan-agent-pane:idle"] != nil)
        #expect(PlanAgentRow.second(subagent, now: PlanScenario.now).hasPrefix("\u{201C}Design ov-503\u{201D} \u{00B7} Not on a lane \u{00B7} Working"))
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

    @Test("The Plan's home draws no pinned Now and Next band: the sections carry the same, and Needs You is not said twice")
    func noBand() async throws {
        let store = try await Self.store(.busy)
        let drawn = await PlanThemePageTests.frames(
            PlanHome(board: store, needsYou: PlanNeedsYou(), onOpen: { _ in }), width: 700, height: 3200)
        #expect(drawn["plan-home"] != nil && drawn["plan-landing"] != nil, "\(drawn.keys.sorted())")
        for id in ["plan-band-now", "plan-band-next", "plan-band-needs-you"] { #expect(drawn[id] == nil, "\(id) is gone") }
        // The sentence is still the whole summary where there is no room for sections.
        let sections = store.plan.sections(needsYou: 2, statuses: store.board.statuses, activity: [:])
        #expect(PlanGlanceSentence(sections: sections).now?.hasPrefix("Train 88 CI failed") == true)
    }

    @Test("A train is Train N with its title under it, and its ladder explains each step")
    func landingWords() async throws {
        let drawn = try await Self.drawn(.busy, width: 600)
        #expect(drawn["plan-train-train-86-label"] != nil && drawn["plan-train-train-86-subtitle"] != nil, "Train 86, then its title")
        #expect(drawn["plan-train-train-88-subtitle"] == nil, "a title that says no more than the label is not said again")
        #expect(drawn["plan-train-lane-team-dir-note"] == nil, "a lane simply aboard says nothing")
        let titles = PlanSections.Kind.allCases.map(\.title) + PlanLadder.steps + TrainPhase.allCases.map(\.word)
        #expect(titles.allSatisfy { !$0.contains("Shipping") && !$0.contains("Picked") && !$0.contains("Gated") })
        #expect(PlanLadder.explanation(0, current: 0, phase: .integrating) == "The full test run, on the runner.")
        #expect(PlanWords.trainExplanation == "Reviewed lanes land on main together, after one round of checks.")
    }

    @Test("With no agent alive the Agents section still says what the day spent, and the Landing ladders line up card to card")
    func ladderAndSpend() async throws {
        let store = try await Self.store(.ladder)
        let sections = store.plan.sections(needsYou: 0, statuses: [:], activity: [:])
        let said = "No agents running \u{00B7} 1.2M tokens in the last 24 hours \u{00B7} \u{2248}$12.00"
        #expect(sections.emptyWords(.agents) == said, "\(sections.emptyWords(.agents) ?? "-")")
        let drawn = try await Self.drawn(.ladder, width: 600)
        #expect(drawn["plan-agents-empty"] != nil)
        // (A point of rounding at most.) The first step is the same width whatever it says: a long phase wraps in its own column.
        var widths: [[CGFloat]] = []
        for phase in [TrainPhase.waitingForPreviousTrain, .waitingForMain, .waitingForCi, .readyToLand] {
            let frames = await PlanThemePageTests.frames(PlanLadderView(phase: phase), width: 560, height: 120)
            widths.append((0..<3).compactMap { frames["plan-ladder-step-\($0)"]?.width })
        }
        #expect(widths.allSatisfy { $0.count == 3 }, "\(widths)")
        for row in widths { #expect(row.allSatisfy { abs($0 - row[0]) <= 1 }, "the steps of one ladder are equal: \(row)") }
        #expect(abs(widths[0][0] - widths[1][0]) <= 1, "and equal from card to card: \(widths)")
    }
}
