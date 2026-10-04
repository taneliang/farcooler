import Foundation
import Testing

@testable import AgentKit

// A failure resolved on the runner clears on the watch within one look
// (ov-239). The watch hears from the relay over `/v1/pulse` only, so the
// pulse answer carries how each finished agent's turn ended, and the watch,
// its complication and the phone's widget feed that to the same
// failure-clearing the Live Activity uses (ov-186).
//
// Serialized: the one test that asks over the wire answers through a static.
@Suite(.serialized)
struct RunnerPulseTurnsTests {
    private let now = Date(timeIntervalSince1970: 1_800_000_000)

    private func failedAgent(_ id: String) -> FleetSnapshot.Agent {
        FleetSnapshot.Agent(
            id: id, label: "claude", machine: "orchard", status: "done", glyph: "✗",
            headline: "claude", line: "The turn failed.", feed: [], rank: 1,
            turnFailed: true, activityChangedAt: now.addingTimeInterval(-600))
    }

    private func snapshot(_ agents: [FleetSnapshot.Agent]) -> FleetSnapshot {
        FleetSnapshot(agents: agents, capturedAt: now.addingTimeInterval(-600), complete: true)
    }

    // MARK: - Decoding

    /// The relay's fixture, the very file its suite pins its answer to.
    ///
    /// Mutation: `decodeTurns` returning `[]`. Red.
    @Test func theRelaysTurnsDecode() throws {
        let turns = RunnerPulse.decodeTurns(try Contracts.data("pulse/turns.json"))
        #expect(
            turns == [
                PulseTurn(
                    terminal: "term-01999a90aa10", failed: false,
                    at: Date(timeIntervalSince1970: 1_791_019_860))
            ])
    }

    /// Old clients are unaffected both ways: this build still reads the
    /// runners from an answer with turns, and an older relay's answer, which
    /// has none, is no turns rather than a failed decode.
    ///
    /// A pin on behavior rather than a mutation catch: either spelling of
    /// `Answer.turns` gives none here, so no mutation of it goes red.
    @Test func anAnswerWithoutTurnsIsNone() throws {
        #expect(RunnerPulse.decode(try Contracts.data("pulse/turns.json")) == [])
        let older = Data(#"{"runners":[{"label":"Studio","heardAgo":1,"beatEvery":300}]}"#.utf8)
        #expect(RunnerPulse.decodeTurns(older) == [])
        #expect(RunnerPulse.decodeTurns(Data(#"{"error":"unauthorized"}"#.utf8)) == [])
    }

    /// One entry this build can't read is dropped alone.
    ///
    /// Mutation: decoding `[PulseTurn]` directly. Red.
    @Test func aBadEntryDoesNotTakeTheRestDown() {
        let json = """
            {"runners":[],"turns":[
              {"terminal":"a","failed":false,"at":1791019860000},
              {"terminal":"b","failed":"maybe","at":1791019860000},
              {"terminal":"c","failed":true,"at":0},
              {"terminal":"d","failed":true,"at":1791019860000}]}
            """
        let turns = RunnerPulse.decodeTurns(Data(json.utf8))
        #expect(turns.map(\.terminal) == ["a", "d"])
    }

    // MARK: - Clearing

    /// Mutation: `clearingFailures(vouchedByPulse:)` returning nil. Red.
    @Test func aNewerSuccessClearsTheMark() throws {
        let before = snapshot([failedAgent("t1")])
        let turns = [PulseTurn(terminal: "t1", failed: false, at: now)]
        let after = try #require(before.clearingFailures(vouchedByPulse: turns, at: now))
        #expect(after.failing == 0)
        #expect(after.agents[0].glyph == "✓")
    }

    /// Mutation: dropping the `at` guard (`said >` in `clearingFailures`). Red
    /// on the older success. Mutation: mapping `failed` to `false` in
    /// `clearingFailures(vouchedByPulse:)`. Red on the still-failed one.
    @Test func anOlderSuccessAndAStillFailedTurnLeaveTheMark() {
        let before = snapshot([failedAgent("t1"), failedAgent("t2")])
        let turns = [
            PulseTurn(terminal: "t1", failed: false, at: now.addingTimeInterval(-3_600)),
            PulseTurn(terminal: "t2", failed: true, at: now),
        ]
        #expect(before.clearingFailures(vouchedByPulse: turns, at: now) == nil)
    }

    /// Mutation: matching every failed agent whatever the terminal. Red.
    @Test func onlyTheNamedAgentIsCleared() throws {
        let before = snapshot([failedAgent("t1"), failedAgent("t2")])
        let turns = [PulseTurn(terminal: "t2", failed: false, at: now)]
        let after = try #require(before.clearingFailures(vouchedByPulse: turns, at: now))
        #expect(after.failedTurns == ["t1"])
    }

    // MARK: - What a surface draws

    /// `settled` is the one place a plan's turns are applied. Mutation:
    /// dropping the clearing from `settled`. Red. Mutation: dropping
    /// `quietened`. Red on the second expectation.
    @Test func settledClearsAndQuietens() {
        var working = FleetSnapshot.Agent(
            id: "w1", label: "claude", machine: "orchard", status: "working", glyph: "●",
            headline: "claude", line: "", feed: [], rank: 0, turnFailed: false,
            activityChangedAt: now, observedAt: now)
        working.runnerAnswering = nil
        let before = snapshot([failedAgent("t1"), working])
        let plan = RunnerPulse.Plan(
            quiet: [], nextLook: nil, unstated: ["w1"],
            turns: [PulseTurn(terminal: "t1", failed: false, at: now)])
        let drawn = before.settled(by: plan, at: now)
        #expect(drawn.failing == 0)
        #expect(drawn.agents.first { $0.id == "w1" }?.runnerAnswering == false)
        // Nothing to apply is the snapshot as it was.
        #expect(before.settled(by: RunnerPulse.Plan(quiet: [], nextLook: nil), at: now) == before)
    }

    /// The whole path, as a watch looks: the relay answers over the wire with a
    /// success after the failure on the wrist, and what the plan draws has no
    /// failed mark. A failed look draws the mark as it was. Mutations:
    /// `fetch` answering `.answered(pulses)` with no turns, `plan` dropping
    /// `turns`. Red.
    @Test func oneLookClearsAFailureTheWristStillHolds() async {
        let wrist = snapshot([failedAgent("t1")])
        let credential = PulseCredential(relay: "https://pulse.test", token: "t", account: "u")
        TurnsRelay.answer = (
            200,
            #"{"runners":[{"label":"Studio","heardAgo":1,"beatEvery":300}],"turns":[{"terminal":"t1","failed":false,"at":\#(Int(now.timeIntervalSince1970 * 1000))}]}"#
        )
        let plan = await RunnerPulse.look(
            snapshot: wrist, credential: credential, at: now, session: TurnsRelay.session())
        #expect(wrist.settled(by: plan, at: now).failing == 0)

        TurnsRelay.answer = (500, "{}")
        let failed = await RunnerPulse.look(
            snapshot: wrist, credential: credential, at: now, session: TurnsRelay.session())
        #expect(wrist.settled(by: failed, at: now).failing == 1)
    }
}

/// A relay that answers what a test says, apart from `StubbedRelay` so the two
/// suites' statics can't be read by each other.
final class TurnsRelay: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var answer: (Int, String) = (200, "{}")

    static func session() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [TurnsRelay.self]
        return URLSession(configuration: configuration)
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let (status, body) = Self.answer
        let response = HTTPURLResponse(
            url: request.url!, statusCode: status, httpVersion: nil, headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
