import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Each kind of note in a task's record reads as its own (ov-82).
struct TaskNoteStyleTests {
    @Test("Every kind has a label, one icon family and a style")
    func everyKindHasALabelAnIconAndAStyle() {
        let expected: [TaskNoteKind: (String, String, TaskNoteStyle.Tint, TaskNoteStyle.Weight)] = [
            .decision: ("Decision", "checkmark.seal", .accent, .prominent),
            .finding: ("Finding", "lightbulb", .primary, .standard),
            .progress: ("Progress", "chart.bar", .secondary, .standard),
            .question: ("Question", "questionmark.bubble", .orange, .standard),
            .answer: ("Answer", "text.bubble", .green, .standard),
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

    @Test("An answer right after a question is paired with it")
    func anAnswerAfterAQuestionIsPaired() {
        func note(_ id: String, _ kind: TaskNoteKind) -> TaskNoteRow {
            TaskNoteRow(id: id, kind: kind, actor: "user", at: .now, body: "")
        }
        let notes = [note("1", .question), note("2", .answer), note("3", .answer), note("4", .comment), note("5", .answer)]
        #expect(TaskNoteStyle.answersPaired(notes) == ["2"])
    }

    @Test("The text above the divider keeps a minimum height")
    func theTextKeepsAMinimumHeight() {
        #expect(TaskColumnModel.topHeight(total: 700, share: 0.15, minimum: 260) == 260)
        #expect(TaskColumnModel.topHeight(total: 700, share: 0.5, minimum: 260) == 350)
        // Never into the bottom's own minimum, in a window too short for both.
        #expect(TaskColumnModel.topHeight(total: 200, share: 0.4, minimum: 260) == 170)
    }
}
