import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Where a hunk begins, where it ends, and what it first changed.
///
/// In the target with teeth because none of it is visible by looking. What
/// these numbers become is a sentence in a prompt — "In `push.ts`, around lines
/// 120-148" — and a wrong one sends an agent to read the wrong twenty lines of
/// somebody's branch and act on them. On screen the marker looks identical
/// either way: `@@` and two numbers taken from the hunk's first row, which is
/// the half that was already right.
struct DiffHunkMarkTests {
    private func line(
        _ kind: DiffComputation.Kind, old: Int?, new: Int?, _ text: String = "x"
    ) -> DiffComputation.Line {
        DiffComputation.Line(id: 0, kind: kind, oldNumber: old, newNumber: new, text: text)
    }

    /// Three context lines, a change, three more — one hunk, running the whole
    /// length of what was given.
    @Test func oneHunkRunsFromItsFirstLineToItsLast() {
        let lines = [
            line(.context, old: 10, new: 10),
            line(.removed, old: 11, new: nil, "old text"),
            line(.added, old: nil, new: 11, "new text"),
            line(.context, old: 12, new: 12),
        ]
        let marks = DiffHunkMark.marks(in: lines)
        #expect(marks.count == 1)
        #expect(marks[0].firstNew == 10)
        #expect(marks[0].lastNew == 12)
        #expect(marks[0].quote == "old text")
    }

    /// The rule the view has always used to find a gap is the rule that ends a
    /// hunk: two lines whose new-side numbers are not consecutive have the ones
    /// between them missing.
    @Test func aJumpInLineNumbersStartsAnotherHunk() {
        let lines = [
            line(.context, old: 1, new: 1),
            line(.added, old: nil, new: 2, "first change"),
            line(.context, old: 2, new: 3),
            // …ninety-six unchanged lines the daemon did not send…
            line(.context, old: 98, new: 99),
            line(.removed, old: 99, new: nil, "second change"),
            line(.context, old: 100, new: 100),
        ]
        let marks = DiffHunkMark.marks(in: lines)
        #expect(marks.count == 2)
        #expect(marks[0].firstNew == 1)
        #expect(marks[0].lastNew == 3)
        #expect(marks[0].quote == "first change")
        #expect(marks[1].index == 2)
        #expect(marks[1].firstNew == 99)
        #expect(marks[1].lastNew == 100)
        #expect(marks[1].quote == "second change")
    }

    /// The case the anchor's `firstNew` exists for. A hunk that opens with a
    /// removed line has no new-side number on its first row — which is what the
    /// marker prints — and an anchor built from that would say "the whole file"
    /// while the reader was pointing at one hunk of it.
    @Test func aHunkThatOpensWithARemovalStillKnowsWhereItIs() {
        let lines = [
            line(.removed, old: 40, new: nil, "gone"),
            line(.context, old: 41, new: 40),
            line(.context, old: 42, new: 41),
        ]
        let marks = DiffHunkMark.marks(in: lines)
        #expect(marks[0].new == nil)
        #expect(marks[0].firstNew == 40)
        #expect(marks[0].lastNew == 41)
    }

    /// A file whose only change is a deletion at its end has no extent in the
    /// new file at all, and says so rather than inventing one.
    @Test func aHunkThatIsNothingButRemovalsHasNoNewSidePlace() {
        let marks = DiffHunkMark.marks(in: [
            line(.removed, old: 7, new: nil, "gone"),
            line(.removed, old: 8, new: nil, "also gone"),
        ])
        #expect(marks.count == 1)
        #expect(marks[0].firstNew == nil)
        #expect(marks[0].lastNew == nil)
        #expect(marks[0].quote == "gone")
    }

    /// The quote is the first CHANGED line, not the first line: a hunk opens
    /// with context, and quoting an untouched line back at an agent points it
    /// at the line before the thing being talked about.
    @Test func theQuoteSkipsTheContextAHunkOpensWith() {
        let marks = DiffHunkMark.marks(in: [
            line(.context, old: 1, new: 1, "unchanged"),
            line(.context, old: 2, new: 2, "also unchanged"),
            line(.added, old: nil, new: 3, "the point"),
        ])
        #expect(marks[0].quote == "the point")
    }

    @Test func noLinesMeansNoHunks() {
        #expect(DiffHunkMark.marks(in: []).isEmpty)
    }
}
