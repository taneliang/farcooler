import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Each kind of note in a task's record reads as its own (ov-82).
struct TaskNoteStyleTests {
    @Test("Every kind has a label, one icon family and a style")
    func everyKindHasALabelAnIconAndAStyle() {
        let expected: [TaskNoteKind: (String, String, TaskNoteStyle.Tint, TaskNoteStyle.Weight)] = [
            .decision: ("Decision", "checkmark.seal", .secondary, .prominent),
            .finding: ("Finding", "lightbulb", .secondary, .standard),
            .progress: ("Progress", "chart.bar", .secondary, .standard),
            .question: ("Question", "questionmark.bubble", .secondary, .standard),
            .answer: ("Answer", "text.bubble", .secondary, .standard),
            .comment: ("Comment", "bubble.left", .secondary, .standard),
            .statusChange: ("Status Change", "arrow.right.circle", .secondary, .quiet),
            .created: ("Created", "plus.circle", .secondary, .quiet),
        ]
        #expect(expected.count == TaskNoteKind.allCases.count)
        for kind in TaskNoteKind.allCases {
            let style = TaskNoteStyle.of(kind)
            let want = expected[kind]!
            #expect(style == TaskNoteStyle(label: want.0, symbol: want.1, tint: want.2, weight: want.3))
        }
        #expect(Set(TaskNoteKind.allCases.map { TaskNoteStyle.of($0).symbol }).count == TaskNoteKind.allCases.count)
    }

    /// Color only for what needs the person (ov-98, owner): every kind's
    /// label is secondary, and the one accent in the record is a question
    /// still waiting on its answer, in a task waiting on a decision. One
    /// answered, or in a task that has moved on, is history.
    @Test("Only an unanswered question that waits on you is in the accent")
    func onlyAnUnansweredQuestionIsAccented() {
        func note(_ id: String, _ kind: TaskNoteKind) -> TaskNoteRow {
            TaskNoteRow(id: id, kind: kind, actor: "manager", at: .now, body: "")
        }
        // Display order, newest first: the open question on top, an
        // answered one under it.
        let notes = [note("q2", .question), note("q1", .question), note("a1", .answer), note("d", .decision)]
        func tint(_ id: String, _ status: TaskStatus) -> TaskNoteStyle.Tint {
            TaskNoteStyle.tint(of: notes.first { $0.id == id }!, in: notes, status: status)
        }
        #expect(tint("q2", .needsDecision) == .accent)
        #expect(tint("q1", .needsDecision) == .secondary, "answered, and still lit")
        #expect(tint("q2", .inProgress) == .secondary, "the task moved on, and it's still lit")
        #expect(tint("a1", .needsDecision) == .secondary)
        #expect(tint("d", .needsDecision) == .secondary)
    }

    @Test("Machine-written kinds are quiet, and only the decision is prominent")
    func machineWrittenKindsAreQuiet() {
        for kind in TaskNoteKind.allCases {
            let weight = TaskNoteStyle.of(kind).weight
            #expect((weight == .quiet) == kind.isMachineWritten)
            #expect((weight == .prominent) == (kind == .decision))
        }
    }

    @Test("A decision's rejected options come apart from what was chosen")
    func aDecisionsRejectedOptionsComeApart() {
        let split = TaskNoteStyle.decision("Use WebKit. Rejected: (a) PDFKit; (b) wkhtmltopdf")
        #expect(split.chosen == "Use WebKit.")
        #expect(split.rejected == "(a) PDFKit; (b) wkhtmltopdf")
        #expect(TaskNoteStyle.decision("Use WebKit.") == .init(chosen: "Use WebKit.", rejected: nil))
        #expect(TaskNoteStyle.decision("Rejected: x").rejected == nil)
    }

    @Test("A decision's rejected options come from extra.rejected on the wire")
    func rejectedOptionsComeFromTheWire() throws {
        let json = """
            {"task": {}, "notes": [{"id": "n1", "kind": "decision", "actor": "manager", "at": 0,
              "body": "Two columns.", "extra": {"rejected": ["An inspector", "Tabs"]}},
             {"id": "n2", "kind": "decision", "actor": "manager", "at": 0, "body": "Plain.", "extra": "junk"}]}
            """
        let notes = try TaskDetailModel.decode(Data(json.utf8)).notes
        #expect(notes[0].rejected == ["An inspector", "Tabs"])
        #expect(TaskNoteStyle.decision(notes[0]) == .init(chosen: "Two columns.", rejected: "An inspector; Tabs"))
        #expect(notes[1].rejected.isEmpty)
        #expect(TaskNoteStyle.decision(notes[1]).rejected == nil)
    }

    @Test("An answer right after a question is paired with it")
    func anAnswerAfterAQuestionIsPaired() {
        func note(_ id: String, _ kind: TaskNoteKind) -> TaskNoteRow {
            TaskNoteRow(id: id, kind: kind, actor: "user", at: .now, body: "")
        }
        let notes = [note("1", .question), note("2", .answer), note("3", .answer), note("4", .comment), note("5", .answer)]
        #expect(TaskNoteStyle.answersPaired(notes) == ["2"])
    }

    @Test("The task view's record is newest first, pairs kept, Created last")
    func theRecordIsNewestFirst() {
        func note(_ id: String, _ kind: TaskNoteKind, _ t: Double) -> TaskNoteRow {
            TaskNoteRow(id: id, kind: kind, actor: "user", at: Date(timeIntervalSince1970: t), body: "")
        }
        let feed = TaskNoteStyle.feed([
            note("made", .created, 0), note("q", .question, 1), note("a", .answer, 2), note("p", .progress, 3),
        ])
        #expect(feed.notes.map(\.id) == ["p", "q", "a", "made"])
        #expect(feed.paired == ["a"])
    }
}
