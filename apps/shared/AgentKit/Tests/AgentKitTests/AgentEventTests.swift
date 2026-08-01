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
