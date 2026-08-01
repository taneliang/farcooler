import Testing
@testable import AgentKit

private func seq(_ n: UInt64, _ e: AgentEvent) -> Sequenced { Sequenced(seq: n, event: e) }

@Test func consecutiveChunksOfOneMessageBecomeOneRow() {
    // The agent streams a sentence as many chunks. One row per chunk would
    // render a column of one-word paragraphs.
    var t = Transcript()
    t.apply([
        seq(0, .message(role: .agent, text: "Hello, ")),
        seq(1, .message(role: .agent, text: "world")),
    ])
    #expect(t.rows.count == 1)
    guard case let .message(role, text) = t.rows[0] else {
        Issue.record("expected one message row")
        return
    }
    #expect(role == .agent)
    #expect(text == "Hello, world")
}

@Test func aRoleChangeStartsANewRow() {
    var t = Transcript()
    t.apply([
        seq(0, .message(role: .agent, text: "a")),
        seq(1, .message(role: .thought, text: "b")),
    ])
    #expect(t.rows.count == 2)
}

@Test func aToolUpdateMutatesItsCallRatherThanAppending() {
    // A tool that reports progress four times must stay one row, or the
    // transcript fills with duplicates of the same call.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "t1", title: "Edit main.rs", kind: "edit", status: .pending, locations: [])),
        seq(1, .toolUpdate(id: "t1", status: .completed, content: nil, diff: nil)),
    ])
    #expect(t.rows.count == 1)
    guard case let .tool(tool) = t.rows[0] else {
        Issue.record("expected a tool row")
        return
    }
    #expect(tool.status == .completed)
}

@Test func aDiffAttachesToTheToolItBelongsTo() {
    var t = Transcript()
    let diff = Diff(path: "main.rs", oldText: "old", newText: "new")
    t.apply([
        seq(0, .toolCall(id: "t1", title: "Edit", kind: "edit", status: .pending, locations: [])),
        seq(1, .toolUpdate(id: "t1", status: .completed, content: nil, diff: diff)),
    ])
    guard case let .tool(tool) = t.rows[0] else {
        Issue.record("expected a tool row")
        return
    }
    #expect(tool.diff == diff)
}

@Test func aGapIsItsOwnRowAndIsNeverMergedAway() {
    // If a gap could merge into a neighbouring message the user would never
    // learn that history is missing, which is the one thing this design
    // promises never to hide.
    var t = Transcript()
    t.apply([
        seq(0, .message(role: .agent, text: "a")),
        seq(1, .gap(.ringTrimmed)),
        seq(2, .message(role: .agent, text: "b")),
    ])
    #expect(t.rows.count == 3)
    guard case .gap = t.rows[1] else {
        Issue.record("the gap must survive as its own row")
        return
    }
}

@Test func aPendingPermissionIsExposedAndClearedWhenAnswered() {
    var t = Transcript()
    let options = [PermissionOption(id: "allow", name: "Allow", kind: "allow_once")]
    t.apply([seq(0, .permission(id: "r1", toolCall: "t1", options: options))])
    #expect(t.pendingPermission?.id == "r1")

    t.apply([seq(1, .resolved(id: "r1", chosen: "allow"))])
    #expect(t.pendingPermission == nil)
}

@Test func thePlanIsReplacedWholesaleNotAppended() {
    // The daemon sends the whole plan every time, so appending would show
    // every historical version of the todo list stacked up.
    var t = Transcript()
    t.apply([seq(0, .plan(entries: [PlanEntry(content: "one", priority: "high", status: "pending")]))])
    t.apply([seq(1, .plan(entries: [PlanEntry(content: "two", priority: "high", status: "done")]))])
    #expect(t.plan.count == 1)
    #expect(t.plan[0].content == "two")
}

@Test func theCursorTracksTheHighestSeqSeen() {
    // Reconnect asks for everything after this. An off-by-one repeats a
    // message or skips one.
    var t = Transcript()
    t.apply([seq(0, .message(role: .agent, text: "a")), seq(7, .message(role: .user, text: "b"))])
    #expect(t.cursor == 8)
}

@Test func aTranscriptWithOnlyAGapStillHasSomethingToDraw() {
    // The empty-state and the gap-state are different. Rendering "no messages
    // yet" over a gap would tell the user nothing happened when in fact
    // something happened and was lost.
    var t = Transcript()
    t.apply([Sequenced(seq: 0, event: .gap(.loadUnsupported))])
    #expect(t.rows.count == 1)
    #expect(t.rows.isEmpty == false)
}
