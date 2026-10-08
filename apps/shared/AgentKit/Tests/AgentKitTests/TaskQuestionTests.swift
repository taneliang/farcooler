import Foundation
import Testing

@testable import AgentKit

// The question a task in Needs Decision is waiting on, read out of `task show
// --json`: its words and the options it offered (`task ask --option`), which
// the card draws as buttons (spec §2.5).

private func detail(_ notes: [String]) -> Data {
    Data(#"{"task":{},"notes":[\#(notes.joined(separator: ","))],"blocks":[]}"#.utf8)
}

private func note(_ id: String, _ kind: String, at: Int, body: String = "b", extra: String = "{}")
    -> String
{
    #"{"id":"\#(id)","kind":"\#(kind)","actor":"manager","at":\#(at),"body":"\#(body)","extra":\#(extra)}"#
}

@Test("A task's open question carries its options, in the order they were offered")
func aTasksOpenQuestionCarriesItsOptions() throws {
    let data = detail([
        note("n1", "created", at: 1),
        note("n2", "question", at: 2, body: "Which store?", extra: #"{"options":["SQLite","Postgres"]}"#),
    ])
    let question = try #require(TaskQuestion.open(in: data))
    #expect(question.body == "Which store?")
    #expect(question.options == ["SQLite", "Postgres"])
    #expect(question.id == "n2")
}

/// The latest question is the one waiting; an earlier one, answered or not,
/// isn't. One with no options still asks, with nothing to offer.
@Test("The latest question is the open one, and one with no options has none")
func theLatestQuestionIsTheOpenOne() throws {
    let data = detail([
        note("n1", "question", at: 1, body: "First?", extra: #"{"options":["a"]}"#),
        note("n2", "answer", at: 2, body: "a"),
        note("n3", "question", at: 3, body: "Second?"),
    ])
    let question = try #require(TaskQuestion.open(in: data))
    #expect(question.body == "Second?")
    #expect(question.options.isEmpty)
}

/// An answer written after the question closes it: the card offers no
/// buttons for a question somebody already answered.
@Test("A question answered since is not open")
func aQuestionAnsweredSinceIsNotOpen() {
    let data = detail([
        note("n1", "question", at: 1, body: "Which?", extra: #"{"options":["a","b"]}"#),
        note("n2", "answer", at: 2, body: "b"),
    ])
    #expect(TaskQuestion.open(in: data) == nil)
    #expect(TaskQuestion.open(in: detail([note("n1", "created", at: 1)])) == nil)
    #expect(TaskQuestion.open(in: Data("not json".utf8)) == nil)
}
