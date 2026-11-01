import Foundation
import Testing

@testable import AgentKit

/// Landings (ov-309, ov-872) and the runner's CI reads (ov-306) as the Mac and
/// the iPhone read them: `test/fixtures/plan.json`, which the CLI's and the
/// client's tests write from the wire, and Now drawn as row groups.
struct PlanLandingsTests {
    static let now = PlanModelTests.now

    @Test("plan --json carries landings, CI reads, a theme's spend and the board's counts")
    func decodesTheFixture() throws {
        let plan = try PlanModelTests.fixture()
        let landing = try #require(plan.landings.first)
        #expect(landing.number == 9 && landing.state == .red && landing.pushedSha == "c85bf83d")
        #expect(landing.tickets == [PlanCardRef(task: plan.lines[2].task, key: "ov-4")])
        let read = try #require(plan.ci(of: landing))
        #expect(read.status == .failed && read.jobs.count == 3 && read.sha.hasPrefix("c85bf83d"))
        #expect(plan.themes[0].spend?.totalTokens == 320_000)
        #expect(plan.boardCounts?.inReview == 3 && plan.cardCount("open") == 9 && plan.cardCount("done") == 11)
    }

    @Test("an older answer, with no landings, CI or counts, still reads")
    func anOlderAnswerReads() throws {
        let plan = try PlanModelTests.plan(themes: [], lines: [])
        #expect(plan.landings.isEmpty && plan.ci.isEmpty && plan.boardCounts == nil && plan.cardCount("open") == nil)
    }

    @Test("Now heads each landing's tickets with the landing, then the tickets on none; a pushed landing with no ticket working still shows")
    func nowGroupsByLanding() throws {
        let plan = try PlanModelTests.plan(
            themes: [],
            lines: [
                PlanModelTests.line("t1", "reviewing", landing: 14),
                PlanModelTests.line("t3", "building"),
                PlanModelTests.line("t2", "landing", landing: 14),
                PlanModelTests.line("t9", "landed", landing: 13),
            ],
            landings: [
                PlanModelTests.landing(14, "red", tickets: ["t1", "t2"], sha: "c85bf83d"),
                PlanModelTests.landing(15, "pushed", sha: "1a1b3275"),
                PlanModelTests.landing(13, "landed", tickets: ["t9"]),
            ])
        let groups = plan.nowGroups
        #expect(groups.map { $0.landing?.number } == [14, 15, nil], "the landed one is out of Now")
        #expect(groups.map { $0.lines.map(\.task) } == [["t1", "t2"], [], ["t3"]])
        #expect(PlanWords.status(plan.lines[0]) == "In Review \u{00B7} in Landing 14")
        #expect(plan.liveLandings.map(\.number) == [14, 15])
        #expect(plan.lines(on: plan.landings[2]).map(\.task) == ["t9"], "a landed landing still lists its tickets")
    }

    @Test("a landing's line: its state, the SHA, and CI as it was last read, or that it hasn't been")
    func aLandingSaysItsCI() throws {
        let read: [String: Any] = [
            "subject": "sha:c85bf83dce46a6b71d7312afc623899ae7914658", "sha": "c85bf83dce46a6b71d7312afc623899ae7914658",
            "status": "running", "url": "https://github.com/o/r/actions/runs/1", "fetched_at": Self.now, "changed_at": Self.now,
            "jobs": [["name": "CI / Rust", "state": "running", "url": ""], ["name": "CI / iOS", "state": "passed", "url": ""]],
        ]
        let plan = try PlanModelTests.plan(
            themes: [], lines: [], landings: [PlanModelTests.landing(14, "pushed", sha: "c85bf83d")], ci: [read])
        let landing = plan.landings[0]
        let ci = plan.ci(of: landing)
        #expect(ci != nil, "a short SHA finds the read of its full one")
        #expect(!PlanWords.landingNeedsAttention(landing, ci: ci))
        let red = try PlanModelTests.plan(themes: [], lines: [], landings: [PlanModelTests.landing(15, "red")]).landings[0]
        #expect(PlanWords.landingNeedsAttention(red, ci: nil))
        #expect(PlanWords.landingNeedsAttention(landing, ci: PlanCIRead(subject: "x", status: .failed)), "or CI failed")
        #expect(PlanModelTests.landing(16, "pushed")["ci_subject"] as? String == "", "a landing with no SHA has no CI subject")
        let none = try PlanModelTests.plan(themes: [], lines: [], landings: [PlanModelTests.landing(16, "pushed")])
        #expect(none.ci(of: none.landings[0]) == nil)
    }

    @Test("a read that stopped working says how old it is, on the landing and on a page (review train-1005c M1)")
    func aStaleReadSaysHowOld() throws {
        let read = PlanCIRead(subject: "main", status: .passed, fetchedAt: Self.now - 3 * 3_600_000)
        #expect(PlanWords.ciStale(read, now: Self.now) == "as of 3 h ago")
        #expect(PlanWords.ciStale(PlanCIRead(subject: "main", status: .passed, fetchedAt: Self.now - 60_000), now: Self.now) == nil)
        #expect(PlanWords.ciStale(PlanCIRead(subject: "main", status: .unknown), now: Self.now) == nil, "never read: its status says so")
        let world = PageWorld(plan: PlanModel(nowMs: Self.now, ci: [read]))
        #expect(world.resolve(PageRef(.ci("main"))).status == "Passed · as of 3 h ago")
        #expect(world.statText(PageStat(label: "Main", value: "", ref: PageRef(.ci("main")))).detail == "as of 3 h ago")
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
