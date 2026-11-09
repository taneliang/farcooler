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

    /// What the core says of ticket `n`'s pull request stage.
    static func stage(_ plan: PlanModel, _ n: Int) -> PlanView.PageStage? {
        plan.pages()?.ticket(line(plan, n).task)?.stage
    }

    @Test("Every stage kind the runner sends decodes, and the core words each")
    func everyKindHasWords() throws {
        let plan = try Self.fixture()
        // A building ticket's own stage word says Building once, so its page has no stage line.
        let words = (1...14).map { Self.stage(plan, $0)?.words }
        #expect(
            words == [
                nil, "Agent Review", "Fixing", "Waiting on alice and 2 others",
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
        #expect(Self.line(plan, 11).prStage?.queuePosition == 2)
    }

    @Test("An attention stage leads with the problem; Approved stays in the state line")
    func attentionLeadsWithTheProblem() throws {
        let plan = try Self.fixture()
        for (n, problem) in [(8, "Checks Failing"), (9, "Merge Conflicts")] {
            let line = Self.line(plan, n)
            #expect(Self.stage(plan, n)?.words == problem)
            #expect(Self.stage(plan, n)?.full == "Approved, \(problem)")
            #expect(Self.stage(plan, n)?.row == problem)
            #expect(plan.status(of: line, stage: false) == "Approved", "the gray line")
            #expect(plan.status(of: line) == "Approved, \(problem)", "a line with room for one phrase")
        }
        #expect(Self.stage(plan, 6)?.full == "Changes Requested", "changes requested is its own state")
    }

    @Test("The facts under the words come through: reviewers, checks and the pull request")
    func factsDecode() throws {
        let stage = try #require(Self.line(try Self.fixture(), 4).prStage)
        #expect(stage.reviewers == ["alice", "bob", "org/web"])
        #expect(stage.prNumber == 31 && stage.unresolvedThreads == 2)
        #expect(stage.link?.absoluteString == "https://github.com/acme/app/pull/31")
        #expect(Self.line(try Self.fixture(), 8).prStage?.checks == "failing")
    }

    @Test("Unknown is a runner that couldn't ask, so it never reads as no pull request or as building")
    func unknownIsNotNoPR() throws {
        let plan = try Self.fixture()
        let unknown = Self.line(plan, 14)
        #expect(unknown.prStage?.kind == .unknown)
        #expect(plan.status(of: unknown) == "In Review \u{00B7} PR State Unknown", "added to the state, not in its place")
        #expect(Self.stage(plan, 14) != nil)
        let building = Self.line(plan, 1)
        #expect(Self.stage(plan, 1) == nil, "a building ticket's own stage word says Building once")
        #expect(plan.status(of: building) == "In Review")
        #expect(Self.stage(plan, 14)?.words != "No PR" && Self.stage(plan, 14)?.words != "Building")
    }

    @Test("A stage takes the ticket stage's place on a live ticket; a ticket told nothing reads as it always did")
    func statusUsesTheStage() throws {
        let plan = try Self.fixture()
        #expect(plan.status(of: Self.line(plan, 4)) == "Waiting on alice and 2 others")
        #expect(plan.status(of: Self.line(plan, 6)) == "Changes Requested")
        #expect(plan.status(of: Self.line(plan, 6), stage: false) == "In Review")
        #expect(plan.status(of: Self.line(plan, 11)) == "Queued, Position 2")
        let silent = Self.line(plan, 15)
        #expect(silent.prStage == nil)
        #expect(plan.status(of: silent) == "In Review")
    }

    @Test("Only changes requested, failing checks and conflicts need a person")
    func attention() throws {
        let plan = try Self.fixture()
        let needing = (1...14).filter { Self.stage(plan, $0)?.attention == true }
        #expect(needing == [6, 8, 9])
    }

    @Test("A landed or dropped ticket keeps its own words whatever a stale stage says")
    func finishedTicketsIgnoreTheStage() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/plan-stages.json"))
        for (stage, word) in [("landed", "Landed"), ("dropped", "Dropped")] {
            var object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
            var lines = try #require(object["lines"] as? [[String: Any]])
            lines[3]["stage"] = stage
            object["lines"] = lines
            let plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
            #expect(Self.stage(plan, 4) == nil, "a \(stage) ticket has no stage line")
            #expect(plan.status(of: Self.line(plan, 4)) == word)
        }
    }

    @Test("A card's line names its own pull request's stage after the ticket's stage")
    func taskLineWithStage() throws {
        let plan = try Self.fixture()
        let task = Self.line(plan, 4).task
        #expect(plan.taskLine(task)?.text == "In Review \u{00B7} Waiting on alice and 2 others")
        #expect(plan.taskLine(task)?.stage?.words == "Waiting on alice and 2 others")
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
        #expect(newer.status(of: newer.lines[0]) == "Awaiting the council")
        line["pr_stage"] = ["kind": "needs_a_dance"]
        let silent = try PlanModelTests.plan(themes: [], lines: [line])
        #expect(silent.status(of: silent.lines[0]) == "In Review", "nothing to say, so the stage stands")
    }
}
