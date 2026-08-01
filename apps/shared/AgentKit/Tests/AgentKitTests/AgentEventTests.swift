import Testing
@testable import AgentKit

@Test func anAgentMessageDecodes() throws {
    let json = #"{"Message":{"role":"Agent","text":"hello"}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .message(role, text) = event else {
        Issue.record("expected a message, got \(event)")
        return
    }
    #expect(role == .agent)
    #expect(text == "hello")
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

@Test func anEventFromALaterDaemonBecomesAGapRatherThanAThrow() throws {
    // A client one release behind its daemon must still render the session.
    // Throwing would blank the whole transcript over one unknown event;
    // dropping it would silently shorten it. A gap is the honest third option.
    let json = #"{"SomethingInvented":{"x":1}}"#
    let event = try AgentEvent.decode(from: json)
    #expect(event == .gap(.unparsed))
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
    guard case let .sessionStarted(_, _, modes, model, models, options, _) = event else {
        Issue.record("expected sessionStarted, got \(event)")
        return
    }
    #expect(modes.first?.name == "Manual")
    #expect(model == "haiku")
    #expect(models.first?.name == "Haiku")
    #expect(options.first?.id == "mode")
}
