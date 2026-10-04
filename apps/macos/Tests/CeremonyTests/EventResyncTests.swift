import Foundation
import Testing

@testable import Far_Cooler

/// A live event the app cannot read is news it missed (ov-156).
///
/// Every `kind` arm decoded with `try?` and answered a failure with a bare
/// `return`, so a field the daemon added or retyped silenced every event of
/// that kind while the window went on saying Connected. And the copy from an
/// event to a row is field by field, which shipped four missed-field bugs
/// (`chatCapable`, `exitCode`, `activitySince`, `turnFailed`).
@MainActor
struct EventResyncTests {
    // MARK: - A line that does not decode asks for a full read

    private static func dispatch(_ line: String) -> (missed: Int, heard: Int) {
        var missed = 0
        var heard = 0
        EventStream.dispatch(
            Data(line.utf8), decoder: JSONDecoder(),
            onEvent: { _ in heard += 1 }, onLayout: { _ in heard += 1 },
            onTask: { _ in heard += 1 }, onMissed: { missed += 1 },
            onNotice: { _ in heard += 1 }, onReads: { _ in heard += 1 })
        return (missed, heard)
    }

    @Test(
        "A tracked kind whose body does not decode triggers a resync",
        arguments: [
            // A required field missing, and a field retyped: the two ways a
            // daemon change breaks an old Mac.
            #"{"kind":"terminal","id":"t-1","short":"t1"}"#,
            #"{"kind":"terminal","id":7,"short":"t1","worktree":"w","title":"x","preset":"p","state":"running"}"#,
            #"{"kind":"layout","worktree":"w"}"#,
            #"{"kind":"task","workspace":"w"}"#,
            #"{"kind":"notice"}"#,
            #"{"kind":"reads"}"#,
            // No kind at all, and not JSON.
            #"{"id":"t-1"}"#,
            "not json",
        ])
    func undecodableEventsResync(line: String) {
        let result = Self.dispatch(line)
        #expect(result.missed == 1, "\(line)")
        #expect(result.heard == 0)
    }

    @Test func aWellFormedEventDoesNotResync() {
        let line = """
            {"kind":"terminal","id":"t-1","short":"t1","worktree":"w","title":"x",
             "preset":"claude","state":"running"}
            """
        let result = Self.dispatch(line)
        #expect(result.missed == 0)
        #expect(result.heard == 1)
    }

    @Test func aKindThisAppDoesNotTrackIsStillSkippedQuietly() {
        let result = Self.dispatch(#"{"kind":"something_new","a":1}"#)
        #expect(result.missed == 0)
        #expect(result.heard == 0)
    }

    // MARK: - Every field an event carries lands on the row

    /// One event with every property set to a value no default could be, applied
    /// to a row that has none of them. A property the event carries and
    /// `DaemonClient.apply` forgets stays nil on the row, which fails here by
    /// name, so adding a field to `TerminalEvent` without applying it cannot go
    /// green.
    @Test func everyTerminalEventFieldIsAppliedToTheRow() throws {
        let fleetJSON = #"""
            {"runtime_healthy":true,"live_panes":1,"worktrees":[{"id":"w-156","short":"w","task":"lane",
              "branch":"b","worktree":"/tmp/w","state":"active",
              "terminals":[{"id":"t-156","short":"old","title":"old","preset":"shell","state":"idle","epoch":0}]}]}
            """#
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.fleet = try JSONDecoder().decode(Fleet.self, from: Data(fleetJSON.utf8))

        let event = TerminalEvent(
            id: "t-156", short: "new", worktree: "w-156", title: "new title", preset: "claude",
            state: "running", activity: "working", chatCapable: true, exitCode: 3, exitSignal: 9,
            activitySince: 1234, turnStartedAt: 5678, blockedQuestion: "Allow touch x?",
            feed: ["a line"], line: "3/7 · Testing", subagents: ["Explore"], turnFailed: true,
            noticeTaskId: "task-1")
        client.apply(event)

        let row = try #require(client.fleet.worktrees.first?.terminals.first)
        let rowFields = Dictionary(
            uniqueKeysWithValues: Mirror(reflecting: row).children.compactMap { child in
                child.label.map { ($0, "\(child.value)") }
            })
        // The worktree is where the row lives, not a field of it.
        let notRowFields: Set<String> = ["worktree"]
        for child in Mirror(reflecting: event).children {
            guard let label = child.label, !notRowFields.contains(label) else { continue }
            #expect(rowFields[label] != nil, "TerminalEvent.\(label) has no counterpart on Terminal")
            #expect(
                rowFields[label] == "\(child.value)",
                "TerminalEvent.\(label) was not applied: the row has \(rowFields[label] ?? "nothing")")
        }
        // The fixture sets every property: a field added to the event and not
        // to this fixture would be compared as nil against nil.
        for child in Mirror(reflecting: event).children {
            #expect("\(child.value)" != "nil", "the fixture leaves \(child.label ?? "?") unset")
        }
    }
}
