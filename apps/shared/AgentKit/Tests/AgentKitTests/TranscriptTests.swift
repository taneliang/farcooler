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
    guard case let .message(role, text) = t.rows[0].kind else {
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
        seq(1, .toolUpdate(id: "t1", status: .completed, title: nil, content: nil, diff: nil, locations: [])),
    ])
    #expect(t.rows.count == 1)
    guard case let .tool(tool) = t.rows[0].kind else {
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
        seq(1, .toolUpdate(id: "t1", status: .completed, title: nil, content: nil, diff: diff, locations: [])),
    ])
    guard case let .tool(tool) = t.rows[0].kind else {
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
    guard case .gap = t.rows[1].kind else {
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

@Test func commandsArriveOnTheirOwnEventAndReplaceRatherThanAccumulate() throws {
    // The agent resends its whole menu every turn. Appending would grow the
    // picker without bound, showing every command several times over.
    var t = Transcript()
    t.apply([Sequenced(seq: 0, event: .commandsAvailable(commands: ["init", "review"]))])
    t.apply([Sequenced(seq: 1, event: .commandsAvailable(commands: ["init", "review"]))])
    #expect(t.availableCommands == ["init", "review"])

    // And it is not a gap: nothing was lost, so nothing should be drawn.
    #expect(t.rows.isEmpty)
}

@Test func aCommandsEventDecodesRatherThanBecomingAGap() throws {
    let json = #"{"CommandsAvailable":{"commands":["init"]}}"#
    #expect(try AgentEvent.decode(from: json) == .commandsAvailable(commands: ["init"]))
}

@Test func aNewTurnStartsANewRowRatherThanJoiningTheLastReply() {
    // Two turns' replies are both `.agent` and adjacent, so plain coalescing
    // glues them into one paragraph — "…prints hi to the console.Hello! I'm
    // Claude Code…" — with no seam where a whole turn began.
    var t = Transcript()
    t.apply([
        Sequenced(seq: 0, event: .message(role: .agent, text: "First reply.")),
        Sequenced(seq: 1, event: .turnEnded(reason: "EndTurn")),
        Sequenced(seq: 2, event: .message(role: .agent, text: "Second reply.")),
    ])
    #expect(t.rows.count == 2)
}

@Test func chunksWithinOneTurnStillCoalesce() {
    // The guard must not defeat the streaming case it sits next to.
    var t = Transcript()
    t.apply([
        Sequenced(seq: 0, event: .message(role: .agent, text: "Hello, ")),
        Sequenced(seq: 1, event: .message(role: .agent, text: "world")),
    ])
    #expect(t.rows.count == 1)
}

@Test func askingTheSameThingTwiceMakesTwoRowsWithDifferentIds() {
    // Identity was derived from content, so a repeated question produced two
    // rows sharing an id. ForEach over duplicate ids does not merely look odd:
    // it renders blank bands and repeats rows in the wrong places, which is
    // exactly what a user sees when they scroll back.
    var t = Transcript()
    t.appendLocalUserMessage("what does main.rs do?")
    t.apply([Sequenced(seq: 0, event: .turnEnded(reason: "EndTurn"))])
    t.appendLocalUserMessage("what does main.rs do?")

    #expect(t.rows.count == 2)
    #expect(t.rows[0].id != t.rows[1].id)
    #expect(Set(t.rows.map(\.id)).count == t.rows.count)
}

@Test func aSentMessageAppearsImmediatelyRatherThanVanishing() {
    // The adapter echoes user text only when replaying a loaded session, never
    // during a live turn. Without a local echo the message disappears the
    // instant it is sent.
    var t = Transcript()
    t.appendLocalUserMessage("hello")
    guard case let .message(role, text) = t.rows.first?.kind else {
        Issue.record("expected the user's own message")
        return
    }
    #expect(role == .user)
    #expect(text == "hello")
}

@Test func aSessionTitleIsCarriedAndDrawsNoRow() {
    // It arrived unmodelled and became a Gap, which drew a "history missing"
    // break at the end of every turn for a title nobody had asked for.
    var t = Transcript()
    t.apply([Sequenced(seq: 0, event: .sessionInfo(title: "Say ok."))])
    #expect(t.title == "Say ok.")
    #expect(t.rows.isEmpty)
}

@Test func aToolCallIsRenamedAsItResolves() {
    // A Bash call starts life titled "Terminal" and is renamed to the command
    // it ran; a Read starts as "Read File" and becomes the file. Keeping the
    // placeholder left every row describing a category instead of an action.
    var t = Transcript()
    t.apply([
        Sequenced(seq: 0, event: .toolCall(
            id: "t1", title: "Terminal", kind: "execute", status: .pending, locations: [])),
        Sequenced(seq: 1, event: .toolUpdate(
            id: "t1", status: .completed, title: "echo hello && ls",
            content: "hello\nmain.rs", diff: nil, locations: [])),
    ])
    guard case let .tool(tool) = t.rows.first?.kind else {
        Issue.record("expected a tool row")
        return
    }
    #expect(tool.title == "echo hello && ls")
    #expect(tool.content == "hello\nmain.rs")
}
