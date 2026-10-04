import Foundation
import Testing

@testable import AgentKit

/// How a phone reads the plan (ov-274): every read lands in a state, and a
/// read nobody answers is not a spinner.
struct PlanReadingTests {
    static func fixtureData() throws -> Data {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return try Data(contentsOf: root.appendingPathComponent("test/fixtures/plan.json"))
    }

    @Test("A runner's answer, real bytes from the Rust client, reads into the plan")
    func theWireReadsIn() async throws {
        let data = try Self.fixtureData()
        let state = await PlanReadState.read(runnerCan: true) { data }
        let plan = try #require(state.plan)
        #expect(plan.themes.map(\.name) == ["Visual language"])
        #expect(plan.nextUp.map(\.name) == ["mac-fu3"])
    }

    @Test("A runner that lacks board_plan needs an update, and is never asked")
    func olderRunnerIsNotAsked() async {
        let asked = Asked()
        let state = await PlanReadState.read(runnerCan: false) {
            await asked.mark()
            return Data()
        }
        #expect(state == .needsUpdate)
        #expect(await asked.count == 0)
    }

    @Test("A refusal that names the missing capability needs an update; other failures can't be read")
    func refusals() async {
        struct Unsupported: Error {}
        struct Broken: Error {}
        let unsupported = await PlanReadState.read(runnerCan: nil, isUnsupported: { $0 is Unsupported }) { throw Unsupported() }
        let broken = await PlanReadState.read(runnerCan: nil, isUnsupported: { $0 is Unsupported }) { throw Broken() }
        #expect(unsupported == .needsUpdate)
        #expect(broken == .unavailable)
    }

    @Test("Bytes that aren't a plan are unavailable, not a crash and not loading")
    func garbage() async {
        let state = await PlanReadState.read(runnerCan: true) { Data("not json".utf8) }
        #expect(state == .unavailable)
    }

    @Test("A read nobody answers is unavailable once the wait is up, never loading forever")
    func unanswered() async {
        let started = ContinuousClock.now
        let state = await PlanReadState.read(runnerCan: true, timeout: .milliseconds(80)) {
            try await Task.sleep(for: .seconds(60))
            return Data()
        }
        #expect(state == .unavailable)
        #expect(ContinuousClock.now - started < .seconds(5), "it waited out the runner instead of the timeout")
    }

    @Test(
        "A runner that never answers, through a call nothing can cancel, is still unavailable once the wait is up",
        .timeLimit(.minutes(1)))
    func unansweredAndUncancellable() async {
        let started = ContinuousClock.now
        // The client core's own shape: a continuation held under a ticket that
        // cancellation can't wake, and that nothing ever resumes.
        let state = await PlanReadState.read(runnerCan: true, timeout: .milliseconds(100)) {
            try await withCheckedThrowingContinuation { (_: CheckedContinuation<Data, Error>) in }
        }
        #expect(state == .unavailable)
        #expect(ContinuousClock.now - started < .seconds(5))
    }

    @Test("A late answer after the timeout changes nothing")
    func lateAnswerDropped() async throws {
        let data = try Self.fixtureData()
        let state = await PlanReadState.read(runnerCan: true, timeout: .milliseconds(50)) {
            try await withCheckedThrowingContinuation { (c: CheckedContinuation<Data, Error>) in
                DispatchQueue.global().asyncAfter(deadline: .now() + 0.3) { c.resume(returning: data) }
            }
        }
        #expect(state == .unavailable)
        try await Task.sleep(for: .milliseconds(400))
    }

    @Test("Tasks is the default, kept per board, and a runner without the layer draws tasks whatever was chosen")
    func theChoice() throws {
        let defaults = try #require(UserDefaults(suiteName: "plan-reading-\(UUID().uuidString)"))
        #expect(!PlanChoice.shown(host: "h", workspace: "w", defaults: defaults))
        PlanChoice.set(true, host: "h", workspace: "w", defaults: defaults)
        #expect(PlanChoice.shown(host: "h", workspace: "w", defaults: defaults))
        #expect(!PlanChoice.shown(host: "h", workspace: "other", defaults: defaults))
        #expect(PlanChoice.showing(runnerKeepsPlan: true, chosen: true))
        #expect(!PlanChoice.showing(runnerKeepsPlan: false, chosen: true))
        #expect(!PlanChoice.showing(runnerKeepsPlan: true, chosen: false))
    }

    @Test("An outcome gets three lines; progress leaves canceled cards out")
    func theRulings() throws {
        #expect(PlanWords.outcomeLines == 3)
        var counts = PlanCounts()
        counts.done = 4
        counts.backlog = 14
        counts.cancelled = 5
        #expect(PlanWords.progress(counts) == "4 of 18 done")
    }
}

private actor Asked {
    var count = 0
    func mark() { count += 1 }
}

struct PlanNewsTests {
    @Test("The notice the Rust client writes names its board")
    func readsTheCoreLine() throws {
        let line = #"{"event":"plan","workspace":"0192F3A4-0000-7000-8000-000000000001"}"#
        let object = try #require(JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any])
        let notice = try #require(PlanNews(notice: object))
        #expect(notice.touches("0192f3a4-0000-7000-8000-000000000001"))
        #expect(!notice.touches("0192f3a4-0000-7000-8000-000000000002"))
    }

    @Test("Other events, and a plan notice with no board, aren't plan news")
    func ignoresTheRest() {
        #expect(PlanNews(notice: ["event": "task", "workspace": "w"]) == nil)
        #expect(PlanNews(notice: ["event": "plan"]) == nil)
        #expect(PlanNews(notice: ["event": "plan", "workspace": ""]) == nil)
    }

    @Test("The seeded board the phone captures read is a plan, with a record for every theme and lane")
    func theSeededBoardDecodes() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/plan-seeded.json"))
        let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: try #require(object["plan"])))
        let records = try #require(object["records"] as? [String: Any])
        #expect(plan.shownThemes.count == 5)
        #expect(plan.nextUp.count == 4)
        for id in plan.themes.map(\.id) + plan.lanes.map(\.id) {
            let record = try #require(records[id], "no record for \(id)")
            _ = try PlanRecord.decode(JSONSerialization.data(withJSONObject: record))
        }
    }
}
