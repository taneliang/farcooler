import Foundation
import Testing

@testable import AgentKit

/// Trains (ov-309) and the runner's CI reads (ov-306) as the Mac and the
/// iPhone read them: `test/fixtures/plan.json`, which the CLI's and the
/// client's tests write from the wire, and Now drawn as row groups.
struct PlanTrainsTests {
    static let now = PlanModelTests.now

    static func train(_ name: String, _ state: String, lanes: [String], sha: String? = nil) -> [String: Any] {
        [
            "id": "train-\(name)", "short": name, "name": name, "base": "origin/main", "pushed_sha": sha as Any,
            "state": state, "state_since": now - 60_000, "actor": "manager", "created_at": now - 3_600_000,
            "landed_at": NSNull(), "lanes": lanes.map { ["lane": "lane-\($0)", "name": $0] },
            "ci_subject": sha.map { "sha:\($0)" } ?? "",
        ]
    }

    static func plan(trains: [[String: Any]], lanes: [[String: Any]], ci: [[String: Any]] = []) throws -> PlanModel {
        let object: [String: Any] = [
            "now_ms": now, "themes": [], "lanes": lanes, "order": [], "cards": [], "trains": trains, "ci": ci,
        ]
        return try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
    }

    @Test("plan --json carries trains, CI reads, a theme's spend and the board's counts")
    func decodesTheFixture() throws {
        let plan = try PlanModelTests.fixture()
        let train = try #require(plan.trains.first)
        #expect(train.name == "integ-9" && train.state == .red && train.pushedSha == "c85bf83d")
        #expect(train.lanes == [PlanTrainLane(lane: plan.lanes[1].id, name: "mac-ux")])
        let read = try #require(plan.ci(of: train))
        #expect(read.status == .failed && read.jobs.count == 3 && read.sha.hasPrefix("c85bf83d"))
        #expect(plan.themes[0].spend?.totalTokens == 320_000)
        #expect(plan.boardCounts?.inReview == 3 && plan.cardCount("open") == 9 && plan.cardCount("done") == 11)
    }

    @Test("an older answer, with no trains, CI or counts, still reads")
    func anOlderAnswerReads() throws {
        let plan = try Self.plan(trains: [], lanes: [])
        #expect(plan.trains.isEmpty && plan.ci.isEmpty && plan.boardCounts == nil && plan.cardCount("open") == nil)
    }

    @Test("Now heads each train's lanes with the train, then the lanes on none; a pushed train with no lane working still shows")
    func nowGroupsByTrain() throws {
        let plan = try Self.plan(
            trains: [
                Self.train("integ-14", "red", lanes: ["mac-ux", "phones"], sha: "c85bf83d"),
                Self.train("integ-15", "pushed", lanes: [], sha: "1a1b3275"),
                Self.train("integ-13", "landed", lanes: ["old"]),
            ],
            lanes: [
                PlanModelTests.lane("mac-ux", "review", cards: [], train: "integ-14"),
                PlanModelTests.lane("solo", "building", cards: []),
                PlanModelTests.lane("phones", "landing", cards: [], train: "integ-14"),
            ])
        let groups = plan.nowGroups
        #expect(groups.map { $0.train?.name } == ["integ-14", "integ-15", nil])
        #expect(groups.map { $0.lanes.map(\.name) } == [["mac-ux", "phones"], [], ["solo"]])
        #expect(PlanWords.status(plan.lanes[0]) == "In Review · in integ-14")
    }

    @Test("a train's line: its state, the SHA, and CI as it was last read, or that it hasn't been")
    func aTrainSaysItsCI() throws {
        let read: [String: Any] = [
            "subject": "sha:c85bf83dce46a6b71d7312afc623899ae7914658", "sha": "c85bf83dce46a6b71d7312afc623899ae7914658",
            "status": "running", "url": "https://github.com/o/r/actions/runs/1", "fetched_at": Self.now, "changed_at": Self.now,
            "jobs": [["name": "CI / Rust", "state": "running", "url": ""], ["name": "CI / iOS", "state": "passed", "url": ""]],
        ]
        let plan = try Self.plan(trains: [Self.train("integ-14", "pushed", lanes: [], sha: "c85bf83d")], lanes: [], ci: [read])
        let train = plan.trains[0]
        let ci = plan.ci(of: train)
        #expect(ci != nil, "a short SHA finds the read of its full one")
        #expect(PlanWords.train(train, ci: ci) == "Pushed · c85bf83d · CI Running · 1 of 2 jobs done")
        #expect(!PlanWords.trainNeedsAttention(train, ci: ci))
        #expect(PlanWords.train(train, ci: nil) == "Pushed · c85bf83d · CI not read yet")
        let red = PlanTrain(
            id: "t", short: "t", name: "t", base: "", pushedSha: nil, state: .red, stateSince: 0, actor: "", createdAt: 0,
            landedAt: nil, lanes: [], ciSubject: "")
        #expect(PlanWords.trainNeedsAttention(red, ci: nil))
    }

    @Test("CI words: each status, and how the jobs stand")
    func ciWords() {
        let jobs = [PlanCIJob(name: "a", state: "failed"), PlanCIJob(name: "b", state: "canceled"), PlanCIJob(name: "c", state: "passed")]
        #expect(PlanWords.ciSummary(PlanCIRead(subject: "main", status: .failed, jobs: jobs)) == "Failed · 1 of 3 jobs failed", "a canceled job isn't a failed one")
        #expect(PlanWords.ciSummary(PlanCIRead(subject: "main", status: .superseded, jobs: [jobs[1], jobs[2]])) == "Superseded · 2 jobs")
        #expect(PlanWords.ciSummary(PlanCIRead(subject: "main", status: .passed, jobs: [jobs[2]])) == "Passed · 1 job")
        #expect(PlanWords.ciSummary(PlanCIRead(subject: "main", status: .none)) == "No Runs Yet")
        #expect(PlanWords.ciSummary(PlanCIRead(subject: "main", status: .unknown)) == "CI Unknown")
    }
}
