import Testing
@testable import AgentKit

private func seq(_ n: UInt64, _ e: AgentEvent) -> Sequenced { Sequenced(seq: n, event: e) }

@Test func consecutiveChunksOfOneMessageBecomeOneRow() {
    // The agent streams a sentence as many chunks. One row per chunk would
    // render a column of one-word paragraphs.
    var t = Transcript()
    t.apply([
        seq(0, .message(role: .agent, text: "Hello, ", parent: nil)),
        seq(1, .message(role: .agent, text: "world", parent: nil)),
    ])
    #expect(t.rows.count == 1)
    guard case let .message(role, text, _) = t.rows[0].kind else {
        Issue.record("expected one message row")
        return
    }
    #expect(role == .agent)
    #expect(text == "Hello, world")
}

@Test func aRoleChangeStartsANewRow() {
    var t = Transcript()
    t.apply([
        seq(0, .message(role: .agent, text: "a", parent: nil)),
        seq(1, .message(role: .thought, text: "b", parent: nil)),
    ])
    #expect(t.rows.count == 2)
}

@Test func aToolUpdateMutatesItsCallRatherThanAppending() {
    // A tool that reports progress four times must stay one row, or the
    // transcript fills with duplicates of the same call.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "t1", title: "Edit main.rs", kind: "edit", status: .pending, locations: [], parent: nil, subagent: false)),
        seq(1, .toolUpdate(id: "t1", status: .completed, title: nil, content: nil, diff: nil, locations: [], parent: nil, subagent: nil)),
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
        seq(0, .toolCall(id: "t1", title: "Edit", kind: "edit", status: .pending, locations: [], parent: nil, subagent: false)),
        seq(1, .toolUpdate(id: "t1", status: .completed, title: nil, content: nil, diff: diff, locations: [], parent: nil, subagent: nil)),
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
        seq(0, .message(role: .agent, text: "a", parent: nil)),
        seq(1, .gap(.ringTrimmed)),
        seq(2, .message(role: .agent, text: "b", parent: nil)),
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
    t.apply([seq(0, .message(role: .agent, text: "a", parent: nil)), seq(7, .message(role: .user, text: "b", parent: nil))])
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
    let menu = [
        AgentChoice(id: "init", name: "init", description: "Set up a project"),
        AgentChoice(id: "review", name: "review", description: "Review the diff"),
    ]
    var t = Transcript()
    t.apply([Sequenced(seq: 0, event: .commandsAvailable(commands: menu))])
    t.apply([Sequenced(seq: 1, event: .commandsAvailable(commands: menu))])
    #expect(t.availableCommands == menu)

    // And it is not a gap: nothing was lost, so nothing should be drawn.
    #expect(t.rows.isEmpty)
}

@Test func aCommandsEventDecodesRatherThanBecomingAGap() throws {
    // With its description, which the adapter has always sent and which the
    // picker needs — a list of names is a list you have to already know.
    let json = #"{"CommandsAvailable":{"commands":[{"id":"init","name":"init","description":"Set up a project"}]}}"#
    #expect(
        try AgentEvent.decode(from: json)
            == .commandsAvailable(commands: [
                AgentChoice(id: "init", name: "init", description: "Set up a project")
            ]))
}

@Test func aNewTurnStartsANewRowRatherThanJoiningTheLastReply() {
    // Two turns' replies are both `.agent` and adjacent, so plain coalescing
    // glues them into one paragraph — "…prints hi to the console.Hello! I'm
    // Claude Code…" — with no seam where a whole turn began.
    var t = Transcript()
    t.apply([
        Sequenced(seq: 0, event: .message(role: .agent, text: "First reply.", parent: nil)),
        Sequenced(seq: 1, event: .turnEnded(reason: "EndTurn")),
        Sequenced(seq: 2, event: .message(role: .agent, text: "Second reply.", parent: nil)),
    ])
    #expect(t.rows.count == 2)
}

@Test func chunksWithinOneTurnStillCoalesce() {
    // The guard must not defeat the streaming case it sits next to.
    var t = Transcript()
    t.apply([
        Sequenced(seq: 0, event: .message(role: .agent, text: "Hello, ", parent: nil)),
        Sequenced(seq: 1, event: .message(role: .agent, text: "world", parent: nil)),
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
    guard case let .message(role, text, _) = t.rows.first?.kind else {
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
            id: "t1", title: "Terminal", kind: "execute", status: .pending, locations: [], parent: nil, subagent: false)),
        Sequenced(seq: 1, event: .toolUpdate(
            id: "t1", status: .completed, title: "echo hello && ls",
            content: "hello\nmain.rs", diff: nil, locations: [], parent: nil, subagent: nil)),
    ])
    guard case let .tool(tool) = t.rows.first?.kind else {
        Issue.record("expected a tool row")
        return
    }
    #expect(tool.title == "echo hello && ls")
    #expect(tool.content == "hello\nmain.rs")
}

@Test func choosingAConfigOptionShowsImmediately() {
    // The adapter applies `session/set_config_option` and says nothing back,
    // so a picker that waited for confirmation snapped to its old value and
    // read as a control that does nothing.
    var t = Transcript()
    t.apply([Sequenced(seq: 0, event: .sessionStarted(
        sessionID: "s", agentMode: "default", availableModes: [],
        model: "haiku", availableModels: [],
        configOptions: [ConfigOption(
            id: "model", name: "Model", description: "", category: "model",
            kind: "select", currentValue: "haiku",
            options: [AgentChoice(id: "opus", name: "Opus")])],
        availableCommands: [], backend: "acp"))])

    t.selectConfigOptionLocally(id: "model", value: "opus")
    #expect(t.configOptions.first?.currentValue == "opus")
    #expect(t.model == "opus")
}

@Test func aRedeliveredBatchDoesNotRenderTheConversationTwice() {
    // The daemon numbers by position within an epoch, so a batch that arrives
    // again carries numbers already applied. It happens whenever a delivery
    // races a replay — which is exactly what a pane toggle does — and applying
    // it a second time put every message on screen twice.
    var t = Transcript()
    let batch = [
        Sequenced(seq: 0, event: .message(role: .user, text: "hello", parent: nil)),
        Sequenced(seq: 1, event: .turnEnded(reason: "end_turn")),
        Sequenced(seq: 2, event: .message(role: .agent, text: "hi", parent: nil)),
    ]
    t.apply(batch)
    t.apply(batch)

    let spoken = t.rows.compactMap { row -> String? in
        if case let .message(_, text, _) = row.kind { return text }
        return nil
    }
    #expect(spoken == ["hello", "hi"])
}

@Test func answeringAPermissionTakesTheCardDown() {
    // The agent resumes without acknowledging the request it was blocked on,
    // so a card that waited for a `Resolved` stayed up after the work it was
    // gating had already run.
    var t = Transcript()
    t.apply([Sequenced(seq: 0, event: .permission(
        id: "p1", toolCall: "bash",
        options: [PermissionOption(id: "allow", name: "Allow", kind: "allow_once")]))])
    #expect(t.pendingPermission != nil)

    t.clearPendingPermission()
    #expect(t.pendingPermission == nil)
}


@Test func aSubagentsWorkLivesInsideTheCallThatDispatchedIt() {
    // The bug in one test: before this, the subagent's message and its Read
    // rendered as siblings of the Task row, and the transcript claimed the
    // top-level agent had read the file itself.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(1, .message(role: .agent, text: "I'll read the file.", parent: "task1")),
        seq(2, .toolCall(id: "r1", title: "Read main.rs", kind: "read", status: .completed,
                         locations: [], parent: "task1", subagent: false)),
    ])
    #expect(t.rows.count == 1)
    guard case let .subagent(block) = t.rows[0].kind else {
        Issue.record("expected a subagent block, got \(t.rows[0].kind)")
        return
    }
    #expect(block.children.count == 2)
    guard case let .message(_, text, _) = block.children[0].kind else {
        Issue.record("expected the subagent's message inside the block")
        return
    }
    #expect(text == "I'll read the file.")
}

@Test func aToolInsideABlockIsUpdatedInPlaceRatherThanDuplicated() {
    // The flat lookup found nothing for a nested tool and appended a second,
    // half-built row beside the block — one tool rendering as two.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(1, .toolCall(id: "r1", title: "Read", kind: "read", status: .pending,
                         locations: [], parent: "task1", subagent: false)),
        seq(2, .toolUpdate(id: "r1", status: .completed, title: "Read main.rs",
                           content: "3 lines", diff: nil, locations: [], parent: "task1",
                           subagent: nil)),
    ])
    #expect(t.rows.count == 1)
    guard case let .subagent(block) = t.rows[0].kind else {
        Issue.record("expected a subagent block")
        return
    }
    #expect(block.children.count == 1)
    guard case let .tool(tool) = block.children[0].kind else {
        Issue.record("expected a tool row inside the block")
        return
    }
    #expect(tool.status == .completed)
    #expect(tool.title == "Read main.rs")
}

@Test func twoSubagentsRunningAtOnceDoNotBleedIntoEachOther() {
    // Frames from parallel subagents interleave in one stream. Routing by
    // "most recent block" instead of by parent id would file each line under
    // whichever subagent happened to speak last.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "a", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(1, .toolCall(id: "b", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(2, .message(role: .agent, text: "from a", parent: "a")),
        seq(3, .message(role: .agent, text: "from b", parent: "b")),
        seq(4, .message(role: .agent, text: " still a", parent: "a")),
    ])
    #expect(t.rows.count == 2)
    guard case let .subagent(first) = t.rows[0].kind,
        case let .subagent(second) = t.rows[1].kind
    else {
        Issue.record("expected two blocks")
        return
    }
    guard case let .message(_, textA, _) = first.children[0].kind,
        case let .message(_, textB, _) = second.children[0].kind
    else {
        Issue.record("expected a message in each block")
        return
    }
    #expect(textA == "from a still a")
    #expect(textB == "from b")
}

@Test func aSubagentsWordsNeverJoinTheDispatchingAgentsSentence() {
    // Message chunks coalesce with the previous row. Coalescing across a
    // parent boundary would splice a subagent's sentence onto the end of the
    // agent's own, attributing it to the wrong speaker in the worst way.
    var t = Transcript()
    t.apply([
        seq(0, .message(role: .agent, text: "I'll dispatch.", parent: nil)),
        seq(1, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(2, .message(role: .agent, text: "inside", parent: "task1")),
    ])
    #expect(t.rows.count == 2)
    guard case let .message(_, text, _) = t.rows[0].kind else {
        Issue.record("expected the agent's own message on top")
        return
    }
    #expect(text == "I'll dispatch.")
}

@Test func aFrameWhoseParentWasNeverSeenIsShownRatherThanLost() {
    // A trimmed ring or a partial reload can leave a child with no block to
    // hang from. Rendering it at the top level is honest — nothing is missing
    // but the nesting. Dropping it would shorten the transcript silently, and
    // a gap would claim content was lost when none was.
    var t = Transcript()
    t.apply([seq(0, .message(role: .agent, text: "orphaned", parent: "never-seen"))])
    #expect(t.rows.count == 1)
    guard case let .message(_, text, _) = t.rows[0].kind else {
        Issue.record("expected the orphan to render as a top-level message")
        return
    }
    #expect(text == "orphaned")
}

@Test func anOrphanDoesNotJoinTheAgentsOwnSentence() {
    // Both land at the top level, so the container alone cannot keep them
    // apart — which is why the row carries its parent.
    var t = Transcript()
    t.apply([
        seq(0, .message(role: .agent, text: "mine.", parent: nil)),
        seq(1, .message(role: .agent, text: "theirs.", parent: "never-seen")),
    ])
    #expect(t.rows.count == 2)
}

@Test func aSubagentCutOffMidRunDoesNotRenderAsOneThatSucceeded() {
    // A cancelled turn never delivers the dispatch's completion. Leaving the
    // block alone would show a subagent whose fate nobody knows wearing the
    // same mark as one that reported back.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(1, .turnEnded(reason: "Cancelled")),
    ])
    guard case let .subagent(block) = t.rows[0].kind else {
        Issue.record("expected a subagent block")
        return
    }
    #expect(block.interrupted)
}

@Test func aFinishedBlockCarriesItsSummary() {
    // The numbers a collapsed block shows instead of its contents.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(1, .toolUpdate(id: "task1", status: .completed, title: "Count lines", content: nil,
                           diff: nil, locations: [], parent: nil,
                           subagent: SubagentSummary(
                               agentType: "general-purpose", model: "claude-opus-5[1m]",
                               tokens: 12479, toolUses: 1, durationMs: 4962,
                               status: "completed"))),
    ])
    guard case let .subagent(block) = t.rows[0].kind else {
        Issue.record("expected a subagent block")
        return
    }
    #expect(block.summary?.tokens == 12479)
    #expect(block.tool.status == .completed)
    #expect(!block.interrupted)
}

@Test func anInterruptedBlockStopsCountingAsRunning() {
    // Interruption does not change the tool's status — a cut-off subagent
    // stays `inProgress` forever — so a surface asking the status alone keeps
    // the block auto-expanded and spinning for the rest of the session. The
    // two apps derived this separately once and disagreed within a day, which
    // is why it lives on the model.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(1, .turnEnded(reason: "Cancelled")),
    ])
    guard case let .subagent(block) = t.rows[0].kind else {
        Issue.record("expected a subagent block")
        return
    }
    #expect(block.interrupted)
    #expect(block.tool.status == .inProgress, "interruption must not fake a terminal status")
    #expect(!block.isRunning)
}

@Test func aLiveBlockCountsAsRunning() {
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true))
    ])
    guard case let .subagent(block) = t.rows[0].kind else {
        Issue.record("expected a subagent block")
        return
    }
    #expect(block.isRunning)
}

@Test func aSubtitleCountsInEnglish() {
    // Both apps wrote this independently and both said "1 tools". It is one
    // sentence of formatting, which is exactly the kind of thing that gets
    // duplicated and then diverges.
    let one = SubagentBlock(
        tool: ToolRow(id: "t", title: "Task", kind: "think", status: .completed,
                      locations: [], content: nil, diff: nil),
        children: [],
        summary: SubagentSummary(agentType: "general-purpose", model: "m", tokens: 12570,
                                 toolUses: 1, durationMs: 3600, status: "completed"),
        interrupted: false)
    #expect(one.subtitle == "general-purpose · 1 tool · 12k tok · 3.6s")

    let many = SubagentBlock(
        tool: one.tool, children: [],
        summary: SubagentSummary(agentType: "general-purpose", model: "m", tokens: 900,
                                 toolUses: 8, durationMs: 31200, status: "completed"),
        interrupted: false)
    #expect(many.subtitle == "general-purpose · 8 tools · 900 tok · 31.2s")
}

@Test func anInterruptedBlockSaysSoRatherThanReportingNumbers() {
    // An interrupted block can still be carrying a partial summary. Reporting
    // its token count would present a subagent whose outcome nobody knows as
    // one that finished and reported.
    let block = SubagentBlock(
        tool: ToolRow(id: "t", title: "Task", kind: "think", status: .inProgress,
                      locations: [], content: nil, diff: nil),
        children: [],
        summary: SubagentSummary(agentType: "general-purpose", model: "m", tokens: 100,
                                 toolUses: 2, durationMs: 1000, status: "in_progress"),
        interrupted: true)
    #expect(block.subtitle == "interrupted")
}

@Test func aRunningBlockCountsItsStepsInEnglish() {
    let one = SubagentBlock(
        tool: ToolRow(id: "t", title: "Task", kind: "think", status: .inProgress,
                      locations: [], content: nil, diff: nil),
        children: [TranscriptRow(id: 0, kind: .gap(.unparsed))],
        summary: nil, interrupted: false)
    #expect(one.subtitle == "1 step")
}

@Test func aQueuedMessageLeavesTheTranscriptItWasDrawnIn() {
    // Drawn immediately so your words never vanish, then withdrawn once the
    // daemon says it was held: it belongs in the queue, where it can still be
    // edited, not in the conversation it has not joined yet.
    var t = Transcript()
    t.appendLocalUserMessage("use unittest")
    #expect(t.rows.contains { $0.kind == .message(role: .user, text: "use unittest", parent: nil) })

    t.apply([Sequenced(seq: 0, event: .promptQueue(items: [
        QueuedPrompt(id: "0", text: "use unittest")
    ]))])

    #expect(!t.rows.contains { $0.kind == .message(role: .user, text: "use unittest", parent: nil) })
    #expect(t.queue.count == 1)
}

@Test func aMessageSentStraightAwayKeepsItsRow() {
    // Nothing queued it, so nothing withdraws it. This is the ordinary path
    // and the one that must not regress.
    var t = Transcript()
    t.appendLocalUserMessage("add tests")
    t.apply([Sequenced(seq: 0, event: .promptQueue(items: []))])
    #expect(t.rows.contains { $0.kind == .message(role: .user, text: "add tests", parent: nil) })
}

@Test func aQueueEntryThatMatchesNothingWithdrawsNothing() {
    // The failure mode is a duplicate row, never a message the user wrote and
    // never saw again.
    var t = Transcript()
    t.appendLocalUserMessage("add tests")
    t.apply([Sequenced(seq: 0, event: .promptQueue(items: [
        QueuedPrompt(id: "0", text: "something else")
    ]))])
    #expect(t.rows.contains { $0.kind == .message(role: .user, text: "add tests", parent: nil) })
}

@Test func sendingTheSameTextTwiceWithOnlyOneHeldWithdrawsExactlyOneRow() {
    // Two identical sends leave two identical unconfirmed echoes and two
    // identical rows. Matching with `contains` instead of consuming a match
    // per queue entry would let one held item cancel both rows, deleting the
    // one that genuinely went out with no queue entry left to show for it.
    var t = Transcript()
    t.appendLocalUserMessage("retry")
    t.appendLocalUserMessage("retry")
    #expect(t.rows.count == 2)

    t.apply([Sequenced(seq: 0, event: .promptQueue(items: [
        QueuedPrompt(id: "0", text: "retry")
    ]))])

    #expect(t.rows.count == 1)
    #expect(t.rows.contains { $0.kind == .message(role: .user, text: "retry", parent: nil) })
}

@Test func aReplayedMessageAfterAnEpochResetSurvivesAStaleEcho() {
    // An unconfirmed echo describes an uncertain send from the epoch that
    // recorded it. Left alive across `resetForNewEpoch`, it can collide with
    // a message replayed into the new epoch — one that genuinely happened —
    // and a promptQueue naming that text would delete restored history that
    // was never in question.
    var t = Transcript()
    t.appendLocalUserMessage("use unittest")
    t.resetForNewEpoch()
    t.apply([Sequenced(seq: 0, event: .message(role: .user, text: "use unittest", parent: nil))])

    t.apply([Sequenced(seq: 1, event: .promptQueue(items: [
        QueuedPrompt(id: "0", text: "use unittest")
    ]))])

    #expect(t.rows.contains { $0.kind == .message(role: .user, text: "use unittest", parent: nil) })
}
