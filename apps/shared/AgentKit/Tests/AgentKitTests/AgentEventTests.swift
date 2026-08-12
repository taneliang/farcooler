import Testing
@testable import AgentKit

@Test func anAgentMessageDecodes() throws {
    let json = #"{"Message":{"role":"Agent","text":"hello"}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .message(role, text, _) = event else {
        Issue.record("expected a message, got \(event)")
        return
    }
    #expect(role == .agent)
    #expect(text == "hello")
}

@Test func aSubagentsMessageDecodesWithItsParent() throws {
    // Without the parent a client cannot tell a subagent's words from the
    // dispatching agent's, which is exactly how they came to be rendered as
    // the same speaker.
    let json = #"{"Message":{"role":"Agent","text":"I'll read the file.","parent":"toolu_01Wnr"}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .message(_, text, parent) = event else {
        Issue.record("expected a message, got \(event)")
        return
    }
    #expect(text == "I'll read the file.")
    #expect(parent == "toolu_01Wnr")
}

@Test func aMessageWithoutAParentStillDecodes() throws {
    // Every event already in SQLite was written before this field existed. If
    // absence threw, every stored transcript would fail to render.
    let json = #"{"Message":{"role":"Agent","text":"hello"}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .message(_, _, parent) = event else {
        Issue.record("expected a message, got \(event)")
        return
    }
    #expect(parent == nil)
}

@Test func aDispatchDecodesAsOne() throws {
    // The Task row and an ordinary tool row are the same event but for this
    // flag, and only the dispatch owns a block.
    let json = """
        {"ToolCall":{"id":"t1","title":"Task","kind":"think","status":"Pending",\
        "locations":[],"subagent":true}}
        """
    let event = try AgentEvent.decode(from: json)
    guard case let .toolCall(_, _, _, _, _, _, subagent) = event else {
        Issue.record("expected a tool call, got \(event)")
        return
    }
    #expect(subagent)
}

@Test func aFinishedDispatchDecodesItsSummary() throws {
    // These are the numbers a collapsed block shows. Dropping them leaves a
    // finished subagent able to say only that it finished.
    let json = """
        {"ToolUpdate":{"id":"t1","status":"Completed","title":null,"content":null,\
        "diff":null,"locations":[],"subagent":{"agent_type":"general-purpose",\
        "model":"claude-opus-5[1m]","tokens":12479,"tool_uses":1,"duration_ms":4962,\
        "status":"completed"}}}
        """
    let event = try AgentEvent.decode(from: json)
    guard case let .toolUpdate(_, _, _, _, _, _, _, summary) = event else {
        Issue.record("expected a tool update, got \(event)")
        return
    }
    #expect(summary?.agentType == "general-purpose")
    #expect(summary?.tokens == 12479)
    #expect(summary?.toolUses == 1)
    #expect(summary?.durationMs == 4962)
}

@Test func aGapDecodesAsAGapAndNotAsNothing() throws {
    // The contract the whole feature rests on. A decoder that silently
    // skipped an event it did not understand would produce a transcript that
    // is wrong and looks complete.
    let json = #"{"Gap":{"reason":"RingTrimmed"}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .gap(reason) = event else {
        Issue.record("expected a gap, got \(event)")
        return
    }
    #expect(reason == .ringTrimmed)
}

@Test func aLoadFailedGapDecodesWithTheAdaptersDetail() throws {
    // The defect this pins: the adapter's own refusal used to reach only a
    // `println!` on the pane's log surface, which chat mode replaces — so it
    // never reached a client at all. It has to survive decoding to reach one.
    let json = #"{"Gap":{"reason":{"LoadFailed":{"detail":"permission denied"}}}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .gap(reason) = event else {
        Issue.record("expected a gap, got \(event)")
        return
    }
    #expect(reason == .loadFailed(detail: "permission denied"))
}

@Test func aLoadEmptyGapDecodesAsTheUnitVariantItIs() throws {
    let json = #"{"Gap":{"reason":"LoadEmpty"}}"#
    let event = try AgentEvent.decode(from: json)
    #expect(event == .gap(.loadEmpty))
}

@Test func anEventFromALaterDaemonBecomesAGapRatherThanAThrow() throws {
    // A client one release behind its daemon must still render the session.
    // Throwing would blank the whole transcript over one unknown event;
    // dropping it would silently shorten it. A gap is the honest third option.
    let json = #"{"SomethingInvented":{"x":1}}"#
    let event = try AgentEvent.decode(from: json)
    #expect(event == .gap(.unparsed))
}

@Test func anUnknownToolStatusIsNotMistakenForStillRunning() throws {
    // The gap an unknown VARIANT name does not cover: a variant this build
    // knows, carrying an enum value it does not.
    //
    // This used to throw, and `AgentStream` caught it with `try?` inside a
    // `compactMap` — so the whole tool call vanished from the transcript with
    // nothing to mark its place. It must decode, and it must not land on
    // `pending`, which the UI spins on: every status likely to be invented
    // (cancelled, rejected, timed out) is terminal, so pending would leave a
    // finished call spinning forever.
    let json = #"{"ToolCall":{"id":"t1","title":"Read","kind":"read","status":"Cancelled","locations":[]}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .toolCall(_, _, _, status, _, _, _) = event else {
        Issue.record("expected a tool call, got \(event)")
        return
    }
    #expect(status == .unknown)
    #expect(status != .pending, "an unreadable status must not read as still running")
}

@Test func anUnknownRoleStillShowsWhatWasSaid() throws {
    // A role is not worth losing a message over: whatever it is called, it came
    // from the agent's side of the conversation.
    let json = #"{"Message":{"role":"Narrator","text":"hello"}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .message(role, text, _) = event else {
        Issue.record("expected a message, got \(event)")
        return
    }
    #expect(role == .agent)
    #expect(text == "hello")
}

@Test func aRealSessionStartedFromTheDaemonDecodes() throws {
    // The exact bytes a live daemon emitted. A decoder that throws here is not
    // a test failure in the abstract: `AgentStream` drops undecodable events
    // with `try?`, so the chat renders blank — no model selector, no modes, no
    // sign anything is wrong.
    let json = """
    {"SessionStarted":{"session_id":"s","agent_mode":"default",\
    "available_modes":[{"id":"default","name":"Manual","description":"d"}],\
    "model":"haiku",\
    "available_models":[{"id":"haiku","name":"Haiku","description":"fast"}],\
    "config_options":[{"id":"mode","name":"Mode","description":"","category":"mode",\
    "kind":"select","current_value":"default",\
    "options":[{"id":"default","name":"Manual","description":"d"}]}],\
    "available_commands":[]}}
    """
    let event = try AgentEvent.decode(from: json)
    guard case let .sessionStarted(_, _, modes, model, models, options, _, backend) = event else {
        Issue.record("expected sessionStarted, got \(event)")
        return
    }
    #expect(modes.first?.name == "Manual")
    #expect(model == "haiku")
    #expect(models.first?.name == "Haiku")
    #expect(options.first?.id == "mode")
    // These bytes carry no `backend` — they predate the field. Every
    // transcript already in SQLite looks like this, and they were all ACP.
    #expect(backend == "acp")
}

@Test func aNativeSessionSaysWhichProtocolIsCarryingIt() throws {
    // The point of the field: two paths that behave differently — a native
    // backend has no adapter to go stale and can steer into a running turn —
    // should not be indistinguishable in the UI. Without this, telling them
    // apart meant reading `ps`.
    let json = """
    {"SessionStarted":{"session_id":"s","agent_mode":null,\
    "available_modes":[],"model":null,"available_models":[],\
    "config_options":[],"available_commands":[],"backend":"codex"}}
    """
    guard case let .sessionStarted(_, _, _, _, _, _, _, backend) =
        try AgentEvent.decode(from: json)
    else {
        Issue.record("expected sessionStarted")
        return
    }
    #expect(backend == "codex")
}
