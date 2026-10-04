import Foundation
import Testing

@testable import AgentKit

// A failed turn followed by a quiet success clears Failed on the widget and
// the watch (ov-186). They read only the App Group snapshot, and a quiet
// success sends no banner, so the relay's word on the Live Activity is the
// only news there is.

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

struct FleetSnapshotFailureTests {
    @Test func aQuietSuccessTheRelayVouchesForClearsTheFailedMark() throws {
        let before = snapshot([failedAgent("t1")])
        #expect(before.failing == 1)
        let rows = [AgentCardRow(terminal: "t1", status: "done", failed: false)]
        let after = try #require(before.clearingFailures(vouchedBy: rows, at: now))
        #expect(after.failing == 0)
        #expect(after.agents[0].turnFailed == false)
        #expect(after.agents[0].glyph == "✓")
    }

    /// An older relay sends `done` with no word on how it ended, and absent is
    /// not "finished well": the failure stays until something knows better.
    @Test func aRowWithNoWordLeavesTheMarkAlone() {
        let before = snapshot([failedAgent("t1")])
        let rows = [AgentCardRow(terminal: "t1", status: "done", failed: nil)]
        #expect(before.clearingFailures(vouchedBy: rows, at: now) == nil)
    }

    @Test func aStillFailedRowAndAWorkingRowLeaveTheMarkAlone() {
        let before = snapshot([failedAgent("t1"), failedAgent("t2")])
        let rows = [
            AgentCardRow(terminal: "t1", status: "done", failed: true),
            AgentCardRow(terminal: "t2", status: "working", failed: false),
        ]
        #expect(before.clearingFailures(vouchedBy: rows, at: now) == nil)
    }

    @Test func onlyTheAgentTheRowIsAboutIsCleared() throws {
        let before = snapshot([failedAgent("t1"), failedAgent("t2")])
        let rows = [AgentCardRow(terminal: "t2", status: "done", failed: false)]
        let after = try #require(before.clearingFailures(vouchedBy: rows, at: now))
        #expect(after.failedTurns == ["t1"])
    }
}
