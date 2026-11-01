import Foundation
import Testing

@testable import AgentKit

/// Where a ticket's pull request stands, as the Plan view words it
/// (ov-316). The bytes are `test/fixtures/plan-stages.json`, which
/// `plan_stages_json_is_the_shape_the_mac_reads` in `crates/client` writes from
/// the runner's own wire messages, so the decoder is fed what the runner sends.
struct PlanPrStageTests {
    static func fixture() throws -> PlanModel {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return try PlanModel.decode(Data(contentsOf: root.appendingPathComponent("test/fixtures/plan-stages.json")))
    }

    static func line(_ plan: PlanModel, _ n: Int) -> PlanTicketLine { plan.lines[n - 1] }

    @Test("Every stage kind the runner sends decodes and has words")
    func everyKindHasWords() throws {
        let plan = try Self.fixture()
        let words = (1...14).map { Self.line(plan, $0).prStage?.words }
        #expect(
            words == [
                "Building", "Agent Review", "Fixing", "Waiting on alice and 2 others",
                "Waiting on maximilian-wolfgang-hohenzollern-schwarzenberg", "Changes Requested",
                "Approved, Checks Running", "Checks Failing", "Merge Conflicts", "Approved",
                "Queued, Position 2", "Merged", "Closed", "PR State Unknown",
            ])
        #expect(Set((1...14).compactMap { Self.line(plan, $0).prStage?.kind }).count == 13, "all thirteen kinds, none lost")
        #expect((1...14).compactMap { Self.line(plan, $0).prStage?.kind } == [
            .building, .agentReview, .fixing, .waitingOnReviewer, .waitingOnReviewer, .changesRequested,
            .approvedChecksRunning, .approvedChecksFailing, .approvedConflicts, .approved, .queued, .merged, .closed, .unknown,
        ], "every kind sits on its own ticket's pr_stage, in the order the runner sends")
        #expect(plan.lines.allSatisfy { $0.stage == .reviewing }, "the stage is the line's; the pull request's is its pr_stage")
        #expect(Self.line(plan, 11).prStage?.queuePosition == 2 && Self.line(plan, 11).prStage?.words == "Queued, Position 2")
    }

    @Test("An attention stage leads with the problem; Approved stays in the state line")
    func attentionLeadsWithTheProblem() throws {
        let plan = try Self.fixture()
        for (n, problem) in [(8, "Checks Failing"), (9, "Merge Conflicts")] {
            let line = Self.line(plan, n)
            #expect(line.prStage?.words == problem)
            #expect(line.prStage?.stateWords == "Approved")
            #expect(line.prStage?.fullWords == "Approved, \(problem)")
            #expect(PlanWords.status(line, stage: false) == "Approved", "the gray line")
            #expect(PlanWords.status(line) == "Approved, \(problem)", "a line with room for one phrase")
        }
        #expect(Self.line(plan, 6).prStage?.stateWords == nil, "changes requested is its own state")
    }

    @Test("The facts under the words come through: reviewers, checks, the pull request and its threads")
    func factsDecode() throws {
        let stage = try #require(Self.line(try Self.fixture(), 4).prStage)
        #expect(stage.reviewers == ["alice", "bob", "org/web"])
        #expect(stage.prNumber == 31 && stage.unresolvedThreads == 2)
        #expect(stage.detail == "PR 31 \u{00B7} 2 threads open")
        #expect(stage.link?.absoluteString == "https://github.com/acme/app/pull/31")
        #expect(Self.line(try Self.fixture(), 8).prStage?.checks == "failing")
        #expect(Self.line(try Self.fixture(), 5).prStage?.detail == "PR 32", "no threads counted, none said")
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
        let unknown = Self.line(plan, 14)
        #expect(unknown.prStage?.kind == .unknown)
        #expect(PlanWords.status(unknown) == "In Review \u{00B7} PR State Unknown", "added to the state, not in its place")
        #expect(unknown.shownStage != nil)
        let building = Self.line(plan, 1)
        #expect(building.shownStage == nil, "a building ticket's own stage word says Building once")
        #expect(PlanWords.status(building) == "In Review")
        #expect(unknown.prStage?.words != "No PR" && unknown.prStage?.words != "Building")
    }

    @Test("A stage takes the ticket stage's place on a live ticket; a ticket told nothing reads as it always did")
    func statusUsesTheStage() throws {
        let plan = try Self.fixture()
        #expect(PlanWords.status(Self.line(plan, 4)) == "Waiting on alice and 2 others")
        #expect(PlanWords.status(Self.line(plan, 6)) == "Changes Requested")
        #expect(PlanWords.status(Self.line(plan, 6), stage: false) == "In Review")
        #expect(PlanWords.status(Self.line(plan, 11)) == "Queued, Position 2")
        let silent = Self.line(plan, 15)
        #expect(silent.prStage == nil)
        #expect(PlanWords.status(silent) == "In Review")
    }

    @Test("Only changes requested, failing checks and conflicts need a person")
    func attention() throws {
        let plan = try Self.fixture()
        let needing = (1...14).filter { Self.line(plan, $0).prStage?.needsAttention == true }
        #expect(needing == [6, 8, 9])
    }

    @Test("A landed or dropped ticket keeps its own words whatever a stale stage says")
    func finishedTicketsIgnoreTheStage() throws {
        var line = Self.line(try Self.fixture(), 4)
        line.stage = .landed
        #expect(line.shownStage == nil)
        #expect(PlanWords.status(line) == "Landed")
        line.stage = .dropped
        #expect(line.shownStage == nil && PlanWords.status(line) == "Dropped")
    }

    @Test("A card's line names its own pull request's stage after the ticket's stage")
    func taskLineWithStage() throws {
        let plan = try Self.fixture()
        let task = Self.line(plan, 4).task
        #expect(plan.taskLine(task)?.text == "In Review \u{00B7} Waiting on alice and 2 others")
        #expect(plan.taskLine(task)?.stage?.reviewers.count == 3)
        let building = Self.line(plan, 1).task
        #expect(plan.taskLine(building)?.stage == nil)
        #expect(plan.taskLine(building)?.text == "In Review")
        let silent = Self.line(plan, 15).task
        #expect(plan.taskLine(silent)?.text == "In Review")
    }

    @Test("A runner before the stage sends none, and a kind this build doesn't know falls back to the runner's words")
    func olderAndNewerRunners() throws {
        let old = try PlanModelTests.plan(themes: [], lines: [PlanModelTests.line("t1", "reviewing")])
        #expect(old.lines[0].prStage == nil)
        var line = PlanModelTests.line("t1", "reviewing")
        line["pr_stage"] = ["kind": "needs_a_dance", "label": "Awaiting the council"]
        let newer = try PlanModelTests.plan(themes: [], lines: [line])
        #expect(newer.lines[0].prStage?.kind == .unspecified)
        #expect(PlanWords.status(newer.lines[0]) == "Awaiting the council")
        line["pr_stage"] = ["kind": "needs_a_dance"]
        let silent = try PlanModelTests.plan(themes: [], lines: [line])
        #expect(PlanWords.status(silent.lines[0]) == "In Review", "nothing to say, so the stage stands")
    }
}
