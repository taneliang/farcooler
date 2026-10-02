import Foundation
import Testing

@testable import AgentKit

// The record's order on screen: newest first, a question and its answer kept
// together, the "Created" entry at the bottom. Shared by the Mac and the phone.

private func note(_ id: String, _ kind: TaskNoteKind, at seconds: Double) -> TaskNoteRow {
    TaskNoteRow(
        id: id, kind: kind, actor: "user",
        at: Date(timeIntervalSince1970: 1_000_000 + seconds), body: id)
}

private func ids(_ notes: [TaskNoteRow]) -> [String] { notes.map(\.id) }

@Suite("The record's feed order")
struct TaskNoteFeedTests {
    @Test("Newest first")
    func newestFirst() {
        let feed = TaskNoteFeed.newestFirst([
            note("a", .finding, at: 1), note("b", .progress, at: 2), note("c", .comment, at: 3),
        ])
        #expect(ids(feed) == ["c", "b", "a"])
    }

    @Test("Created sits last, even when its time is the newest")
    func createdLast() {
        let feed = TaskNoteFeed.newestFirst([
            note("a", .progress, at: 1), note("made", .created, at: 5), note("b", .progress, at: 3),
        ])
        #expect(ids(feed) == ["b", "a", "made"])
    }

    @Test("A question stays above its answer")
    func pairStaysTogether() {
        let feed = TaskNoteFeed.newestFirst([
            note("q", .question, at: 1), note("ans", .answer, at: 2), note("p", .progress, at: 3),
        ])
        #expect(ids(feed) == ["p", "q", "ans"])
        #expect(TaskNoteFeed.answersPaired(feed) == ["ans"])
    }

    @Test("A pair is ordered by its answer's time")
    func pairByNewestActivity() {
        let feed = TaskNoteFeed.newestFirst([
            note("q", .question, at: 1), note("ans", .answer, at: 9), note("p", .progress, at: 5),
        ])
        #expect(ids(feed) == ["q", "ans", "p"])
    }

    @Test("An unanswered question is ordered by its own time")
    func openQuestion() {
        let feed = TaskNoteFeed.newestFirst([
            note("p", .progress, at: 1), note("q", .question, at: 2),
        ])
        #expect(ids(feed) == ["q", "p"])
    }

    @Test("Equal times keep the later-written note on top")
    func ties() {
        let feed = TaskNoteFeed.newestFirst([
            note("a", .progress, at: 1), note("b", .progress, at: 1),
        ])
        #expect(ids(feed) == ["b", "a"])
    }
}
