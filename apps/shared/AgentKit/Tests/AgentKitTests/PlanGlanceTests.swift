import Foundation
import Testing

@testable import AgentKit

/// The plan on the glance (ov-310): what the watch, the widgets and the Live
/// Activity read off the relay, and what they say.
struct PlanGlanceTests {
    /// The board the relay's fixtures carry, spelled out rather than read
    /// back off a fixture.
    static let main = PlanGlance(
        workspace: "Main", needsYou: 2,
        now: [.init(name: "mac-ux", state: .review), .init(name: "ov-310", state: .building)],
        next: "mac-fu3")

    @Test("The relay's pulse answer carries the board the watch draws")
    func thePulseCarriesThePlan() throws {
        let data = try Contracts.data("pulse/plan.json")
        #expect(RunnerPulse.decodePlan(data) == Self.main)
        #expect(RunnerPulse.decode(data) == [], "the runners still decode beside it")
        // An account with no plan, and every relay before ov-310: none.
        #expect(RunnerPulse.decodePlan(try Contracts.data("pulse/response.json")) == nil)
    }

    @Test("The card the relay moves with a count notice carries the plan, and keeps it across ActivityKit's round trip")
    func theCardCarriesThePlan() throws {
        let update = try #require(try Contracts.object("live-activity/running/plan.json")["aps"] as? [String: Any])
        #expect(update["event"] as? String == "update")
        let state = try JSONSerialization.data(withJSONObject: try #require(update["content-state"]))
        let card = try JSONDecoder().decode(AgentCardState.self, from: state)
        #expect(card.plan == Self.main)
        #expect(card.needsYou == 2)
        let again = try JSONDecoder().decode(AgentCardState.self, from: JSONEncoder().encode(card))
        #expect(again.plan == Self.main)

        // The card before it says no plan, and a glance of the wrong shape
        // costs the glance, never the card.
        let before = try #require(try Contracts.object("live-activity/running/update.json")["aps"] as? [String: Any])
        let older = try JSONSerialization.data(withJSONObject: try #require(before["content-state"]))
        #expect(try JSONDecoder().decode(AgentCardState.self, from: older).plan == nil)
        let broken = Data(#"{"status":"working","detail":"","plan":{"needsYou":1}}"#.utf8)
        let survived = try JSONDecoder().decode(AgentCardState.self, from: broken)
        #expect(survived.plan == nil)
        #expect(survived.status == "working")
    }

    @Test("A lane of the wrong shape costs the lane, and a state from later says Unknown")
    func lenient() throws {
        let raw = #"{"workspace":"Main","needsYou":-3,"now":[{"name":"a","state":"soaking"},{"state":"building"}],"next":""}"#
        let glance = try JSONDecoder().decode(PlanGlance.self, from: Data(raw.utf8))
        #expect(glance == PlanGlance(workspace: "Main", needsYou: 0, now: [.init(name: "a", state: .unknown)]))
        #expect(glance.nowLine == "Now: a Unknown")
    }

    @Test("What a glance says, and leaves unsaid")
    func words() {
        #expect(Self.main.heading == "Main · 2 need you")
        #expect(Self.main.nowLine == "Now: mac-ux In Review, ov-310 Building")
        #expect(Self.main.nextLine == "Next: mac-fu3")
        #expect(PlanGlance.words(Self.main.now[0]) == "mac-ux · In Review")
        #expect(Self.main.spoken == "Main board, 2 need you. Now: mac-ux, in review; ov-310, building. Next up: mac-fu3.")

        let quiet = PlanGlance(workspace: "Main", needsYou: 0, now: [])
        #expect(quiet.heading == "Main", "never \"0 need you\"")
        #expect(quiet.nowLine == nil)
        #expect(quiet.nextLine == nil)
        #expect(PlanGlance(workspace: "Main", needsYou: 1, now: []).heading == "Main · 1 needs you")
    }

    /// The runner counts a board's needs-you items and its themes asking the
    /// owner (`plan_glance.rs`); that is the Mac's one number for the board
    /// once its list is read (ov-321). Pinned here beside the daemon's own
    /// test of the same three items and one ask.
    @Test("The glance's count is the Mac's count for the board")
    func theCountIsTheMacs() {
        #expect(WorkspaceNeedsYou.count(items: 2, columnCount: 5, listRead: true, listServed: true, themeAsks: 1) == 3)
    }

    @Test("A plan keeps the widget looking, and rides the look's plan")
    func thePlanRidesTheLook() {
        let now = Date(timeIntervalSince1970: 1_791_019_800)
        let snapshot = FleetSnapshot(agents: [], capturedAt: now, complete: true)
        let look = RunnerPulse.plan(snapshot: snapshot, reading: .answered([], plan: Self.main), at: now)
        #expect(look.glance == Self.main)
        #expect(look.nextLook == now.addingTimeInterval(RunnerPulse.lookEvery))
        let none = RunnerPulse.plan(snapshot: snapshot, reading: .answered([]), at: now)
        #expect(none.glance == nil)
        #expect(none.nextLook == nil, "nothing beating and no plan: today's `.never`")
    }
}
