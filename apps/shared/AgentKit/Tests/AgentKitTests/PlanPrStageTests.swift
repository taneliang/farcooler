import Foundation
import Testing

@testable import AgentKit

/// Where a lane's or a card's pull request stands, as the Plan view words it
/// (ov-316). The bytes are `test/fixtures/plan-stages.json`, which
/// `plan_stages_json_is_the_shape_the_mac_reads` in `crates/client` writes from
/// the runner's own wire messages, so the decoder is fed what the runner sends.
struct PlanPrStageTests {
    static func fixture() throws -> PlanModel {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return try PlanModel.decode(Data(contentsOf: root.appendingPathComponent("test/fixtures/plan-stages.json")))
    }

    static func lane(_ plan: PlanModel, _ n: Int) -> PlanLane { plan.lanes[n - 1] }

    @Test("Every stage kind the runner sends decodes and has words")
    func everyKindHasWords() throws {
        let plan = try Self.fixture()
        let words = (1...14).map { Self.lane(plan, $0).stage?.words }
        #expect(
            words == [
                "Building", "Agent Review", "Fixing", "Waiting on alice and 2 others",
                "Waiting on maximilian-wolfgang-hohenzollern-schwarzenberg", "Changes Requested",
                "Approved, Checks Running", "Checks Failing", "Merge Conflicts", "Approved",
                "Queued, Position 2", "Merged", "Closed", "PR State Unknown",
            ])
        #expect(Set((1...14).compactMap { Self.lane(plan, $0).stage?.kind }).count == 13, "all thirteen kinds, none lost")
    }

    @Test("An attention stage leads with the problem; Approved stays in the state line")
    func attentionLeadsWithTheProblem() throws {
        let plan = try Self.fixture()
        for (n, problem) in [(8, "Checks Failing"), (9, "Merge Conflicts")] {
            let lane = Self.lane(plan, n)
            #expect(lane.stage?.words == problem)
            #expect(lane.stage?.stateWords == "Approved")
            #expect(lane.stage?.fullWords == "Approved, \(problem)")
            #expect(PlanWords.status(lane, stage: false) == "Approved", "the gray line")
            #expect(PlanWords.status(lane) == "Approved, \(problem)", "a line with room for one phrase")
        }
        #expect(Self.lane(plan, 6).stage?.stateWords == nil, "changes requested is its own state")
    }

    @Test("The facts under the words come through: reviewers, checks, the pull request and its threads")
    func factsDecode() throws {
        let stage = try #require(Self.lane(try Self.fixture(), 4).stage)
        #expect(stage.reviewers == ["alice", "bob", "org/web"])
        #expect(stage.prNumber == 31 && stage.unresolvedThreads == 2)
        #expect(stage.detail == "PR 31 \u{00B7} 2 threads open")
        #expect(stage.link?.absoluteString == "https://github.com/acme/app/pull/31")
        #expect(Self.lane(try Self.fixture(), 8).stage?.checks == "failing")
        #expect(Self.lane(try Self.fixture(), 5).stage?.detail == "PR 32", "no threads counted, none said")
    }

    @Test("Waiting on a login says the login, and nobody asked says a reviewer")
    func waitingOn() {
        #expect(PlanPrStage.waiting(on: []) == "Waiting on a reviewer")
        #expect(PlanPrStage.waiting(on: ["alice"]) == "Waiting on alice")
        #expect(PlanPrStage.waiting(on: ["alice", "bob"]) == "Waiting on alice and bob")
        #expect(PlanPrStage.waiting(on: ["alice", "bob", "carol"]) == "Waiting on alice and 2 others")
    }

    @Test("Unknown is a runner that couldn't ask, so it never reads as no pull request or as building")
    func unknownIsNotNoPR() throws {
        let plan = try Self.fixture()
        let unknown = Self.lane(plan, 14)
        #expect(unknown.stage?.kind == .unknown)
        #expect(PlanWords.status(unknown) == "In Review \u{00B7} PR State Unknown", "added to the state, not in its place")
        #expect(unknown.shownStage != nil)
        let building = Self.lane(plan, 1)
        #expect(building.shownStage == nil, "a building lane's own state word says Building once")
        #expect(PlanWords.status(building) == "In Review")
        #expect(unknown.stage?.words != "No PR" && unknown.stage?.words != "Building")
    }

    @Test("A stage takes the lane state's place on a live lane; a lane told nothing reads as it always did")
    func statusUsesTheStage() throws {
        let plan = try Self.fixture()
        #expect(PlanWords.status(Self.lane(plan, 4)) == "Waiting on alice and 2 others")
        #expect(PlanWords.status(Self.lane(plan, 6)) == "Changes Requested")
        #expect(PlanWords.status(Self.lane(plan, 6), stage: false) == "In Review")
        #expect(PlanWords.status(Self.lane(plan, 11)) == "Queued, Position 2")
        let silent = Self.lane(plan, 15)
        #expect(silent.stage == nil && silent.cards[0].stage == nil)
        #expect(PlanWords.status(silent) == "In Review")
    }

    @Test("Only changes requested, failing checks and conflicts need a person")
    func attention() throws {
        let plan = try Self.fixture()
        let needing = (1...14).filter { Self.lane(plan, $0).stage?.needsAttention == true }
        #expect(needing == [6, 8, 9])
    }

    @Test("A landed or dropped lane keeps its own words whatever a stale stage says")
    func finishedLanesIgnoreTheStage() throws {
        var lane = Self.lane(try Self.fixture(), 4)
        lane.state = .landed
        #expect(lane.shownStage == nil)
        #expect(PlanWords.status(lane) == "Landed")
    }

    @Test("A card's line names its own pull request's stage between the lane and the theme")
    func taskLineWithStage() throws {
        let plan = try Self.fixture()
        let task = Self.lane(plan, 4).cards[0].task
        #expect(plan.taskLine(task)?.text == "In lane lane-4 \u{00B7} Waiting on alice and 2 others")
        #expect(plan.taskLine(task)?.stage?.reviewers.count == 3)
        let building = Self.lane(plan, 1).cards[0].task
        #expect(plan.taskLine(building)?.stage == nil)
        #expect(plan.taskLine(building)?.text == "In lane lane-1")
        let silent = Self.lane(plan, 15).cards[0].task
        #expect(plan.taskLine(silent)?.text == "In lane lane-silent")
    }

    @Test("A runner before the stage sends none, and a kind this build doesn't know falls back to the runner's words")
    func olderAndNewerRunners() throws {
        let old = try PlanModelTests.plan(themes: [], lanes: [PlanModelTests.lane("a", "review", cards: ["t1"])])
        #expect(old.lanes[0].stage == nil && old.lanes[0].cards[0].stage == nil)
        var lane = PlanModelTests.lane("a", "review", cards: ["t1"])
        lane["stage"] = ["kind": "needs_a_dance", "label": "Awaiting the council"]
        let newer = try PlanModelTests.plan(themes: [], lanes: [lane])
        #expect(newer.lanes[0].stage?.kind == .unspecified)
        #expect(PlanWords.status(newer.lanes[0]) == "Awaiting the council")
        lane["stage"] = ["kind": "needs_a_dance"]
        let silent = try PlanModelTests.plan(themes: [], lanes: [lane])
        #expect(PlanWords.status(silent.lanes[0]) == "In Review", "nothing to say, so the state stands")
    }
}
