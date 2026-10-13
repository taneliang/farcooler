import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Plan's sections on the Mac (ov-503, phase 5): each section drawn, what
/// it says empty, the two columns from 960 pt and the one column below, no
/// 680-pt cap, the held train's button, and the folded stranded section. The
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

    @Test("Shipping, In Review, Building, Waiting, Agents and Themes are drawn in the owner's order")
    func sectionsInOrder() async throws {
        let drawn = try await Self.drawn(.busy, width: 600)
        let order = ["plan-shipping", "plan-in-review", "plan-building", "plan-waiting", "plan-on-landed-train", "plan-agents", "plan-themes"]
        let frames = try order.map { try #require(drawn[$0], "\($0) in \(drawn.keys.sorted())") }
        for (a, b) in zip(frames, frames.dropFirst()) { #expect(a.maxY <= b.minY + 0.5, "\(a) then \(b)") }
    }

    @Test("From 960 pt the work in motion is the left column and what is queued the right; below, one column")
    func twoColumns() async throws {
        let wide = try await Self.drawn(.busy, width: 900)
        let shipping = try #require(wide["plan-shipping"])
        let waiting = try #require(wide["plan-waiting"])
        let agents = try #require(wide["plan-agents"])
        #expect(waiting.minX > shipping.maxX, "Waiting is beside Shipping: \(shipping) \(waiting)")
        #expect(abs(waiting.minY - shipping.minY) < 1, "the columns start together")
        #expect(abs(agents.minX - waiting.minX) < 1 && agents.minY >= waiting.maxY, "Agents is under Waiting")
        let themes = try #require(wide["plan-themes"])
        #expect(themes.minY >= max(shipping.maxY, agents.maxY) - 0.5 && themes.width > shipping.width * 1.8, "Themes spans both")
        let narrow = try await Self.drawn(.busy, width: 700)
        let stacked = try #require(narrow["plan-waiting"])
        #expect(stacked.minX == narrow["plan-shipping"]?.minX && stacked.minY > (narrow["plan-building"]?.maxY ?? .infinity) - 0.5, "stacked")
    }

    @Test("There's no 680-pt cap: the sections take a 1,400-pt canvas")
    func noReadingWidth() async throws {
        let drawn = try await Self.drawn(.busy, width: 1400)
        let sections = try #require(drawn["plan-sections"])
        #expect(sections.width > 1300, "\(sections.width)")
    }

    @Test("A held train says who holds it in amber, and its holder has Take Off Train; a ready lane's says Move to Next Train")
    func heldTrain() async throws {
        let drawn = try await Self.drawn(.busy, width: 600)
        #expect(drawn["plan-train-train-86-held"] != nil, "Holding for ios-overwrite")
        #expect(drawn["plan-train-lane-ios-overwrite-move"] != nil, "the holder's button: \(drawn.keys.sorted())")
        #expect(drawn["plan-train-lane-team-dir-move"] == nil, "only the holder has one")
        #expect(drawn["plan-train-train-88-held"] == nil, "a train that isn't held says nothing")
        #expect(PlanTrainLaneRow.button(for: .fixing(round: 2)) == "Take Off Train")
        #expect(PlanTrainLaneRow.button(for: .notReady(.building)) == "Take Off Train")
        #expect(PlanTrainLaneRow.button(for: .ready) == "Move to Next Train")
    }

    @Test("A train whose CI failed reads its phase, commit and jobs; the ladder stands on its step")
    func ciFailedTrain() async throws {
        let store = try await Self.store(.busy)
        let sections = store.plan.sections(needsYou: 0, statuses: [:], activity: [:])
        let card = try #require(sections.shipping.first { $0.train.name == "train-88" })
        let words = PlanTrainCard.facts(card, ci: store.plan.plan.ci(of: card.train), now: sections.now)
        #expect(words.hasPrefix("3f78754f \u{00B7} Failed \u{00B7} 1 of 3 jobs failed"), "\(words)")
        #expect(card.phase == .ciFailed && card.phase.needsAttention)
        #expect(PlanLadder.step(.ciFailed) == 3 && PlanLadder.step(.waitingForCi) == 3 && PlanLadder.step(.landed) == 5)
        #expect(PlanLadder.label(3, current: 3, phase: .ciFailed) == "CI Failed" && PlanLadder.label(0, current: 3, phase: .ciFailed) == "Picked")
        #expect(sections.shipping.first?.train.name == "train-88", "furthest along first")
    }

    @Test("Each section says what it says empty, and Needs You-free quiet boards draw nothing else")
    func emptyStates() async throws {
        let quiet = try await Self.drawn(.quiet, width: 600)
        for id in ["plan-shipping-empty", "plan-waiting-empty"] { #expect(quiet[id] != nil, "\(id) in \(quiet.keys.sorted())") }
        #expect(quiet["plan-in-review"] == nil && quiet["plan-building"] == nil, "In Review and Building say nothing with nothing")
        let none = try await Self.drawn(.emptyNext, width: 600)
        #expect(none["plan-waiting-empty"] != nil && none["plan-shipping-empty"] == nil, "Next is empty; trains are shipping")
        #expect(none["plan-agents"] != nil && none["plan-agents-empty"] == nil)
        let store = try await Self.store(.quiet)
        let sections = store.plan.sections(needsYou: 0, statuses: [:], activity: [:])
        #expect(sections.emptyWords(.shipping) == "No train is shipping." && sections.emptyWords(.agents) == "No agents running.")
        let band = PlanGlanceSentence(sections: sections)
        #expect(band.text == "Nothing running. Next: nothing queued.")
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

    @Test("Agents: a header, working rows by spend with tokens then dollars in columns, and Finished folded")
    func agents() async throws {
        let store = try await Self.store(.busy)
        let fleet = store.plan.sections(needsYou: 0, statuses: [:], activity: [:]).fleet
        #expect(fleet.working.map(\.id).first == "pane:orchestrator")
        let drawn = try await Self.drawn(.busy, width: 600)
        #expect(drawn["plan-agents-header"] != nil && drawn["plan-agents-finished"] != nil)
        let orchestrator = try #require(drawn["plan-agent-pane:orchestrator"])
        let builder = try #require(drawn["plan-agent-agent:b1"])
        #expect(abs(orchestrator.maxX - builder.maxX) < 0.5, "the columns share a right edge")
        #expect(drawn["plan-agent-agent:r1"] == nil, "a finished agent is inside the fold")
        let subagent = try #require(fleet.finished.first { $0.role == .subagent })
        #expect(PlanAgentRow.second(subagent, now: PlanScenario.now).hasPrefix("\u{201C}Design ov-503\u{201D} \u{00B7} Not on a lane"))
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

    @Test("The glance band says Now and Next, and a count of what needs you")
    func band() async throws {
        let store = try await Self.store(.busy)
        let sections = store.plan.sections(needsYou: 2, statuses: store.board.statuses, activity: [:])
        let band = PlanGlanceSentence(sections: sections)
        #expect(band.now == "Train 88 CI failed \u{00B7} 1 more train \u{00B7} 2 building \u{00B7} 2 in review", "\(band.now ?? "")")
        #expect(band.next?.hasPrefix("reps \u{2014} ") == true && band.next?.hasSuffix(", then skill-init") == true)
        #expect(PlanGlanceBand.needsYouWords(1) == "1 needs you" && PlanGlanceBand.needsYouWords(2) == "2 need you")
        let drawn = await PlanThemePageTests.frames(PlanGlanceBand(band: band, needsYou: 2), width: 600)
        #expect(drawn["plan-band-now"] != nil && drawn["plan-band-next"] != nil && drawn["plan-band-needs-you"] != nil)
        let none = await PlanThemePageTests.frames(PlanGlanceBand(band: band, needsYou: 0), width: 600)
        #expect(none["plan-band-needs-you"] == nil)
    }

    @Test("Taking a lane off goes through the CLI as the owner, to the next train")
    func moveArguments() {
        #expect(
            DaemonClient.moveArguments("ios-overwrite", repository: "r", workspace: "w")
                == ["plan", "train", "move", "ios-overwrite", "--to", "next", "--repo", "r", "--workspace", "w", "--actor", "user"])
    }
}
