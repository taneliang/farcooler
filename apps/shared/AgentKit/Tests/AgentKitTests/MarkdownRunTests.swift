import Foundation
import Testing

@testable import AgentKit

// What "selection spans two paragraphs" can be asserted as.
//
// SwiftUI's text selection lives inside a `Text`: `.textSelection(.enabled)` on
// a container makes each of that container's `Text`s separately selectable, and
// there is no API that reports where a selection may reach. Driving a real drag
// headlessly was tried and does not work — an `NSHostingView` in an offscreen
// window of a non-active process never takes first responder, so a synthesized
// `leftMouseDown`/`leftMouseDragged` pair produces no selection to measure, and
// its accessibility tree stays empty with no AX client attached.
//
// So the assertion is the thing selection actually depends on: how many text
// elements the paragraphs are drawn as. Two paragraphs in one `AttributedString`
// is one `Text` is one selection. A test that only checked the modifier was
// present would have passed before the bug was fixed and after it regressed.

@Test func adjacentParagraphsAreDrawnAsOneText() {
    let blocks = Markdown.blocks("First paragraph.\n\nSecond paragraph.\n\nThird.")
    #expect(blocks.count == 3)
    #expect(
        Markdown.runs(blocks) == [
            .prose(["First paragraph.", "Second paragraph.", "Third."])
        ])

    let merged = MarkdownText.merged(["First paragraph.", "Second paragraph."])
    // One string, both paragraphs, in order — with a blank line between them,
    // which is also what a reader who selects across both and copies will get.
    #expect(String(merged.characters) == "First paragraph.\n\nSecond paragraph.")
}

/// Everything that is not a paragraph carries layout a `Text` cannot express —
/// a hanging indent, a rule, a grid, a box — so it must still be its own view.
@Test func structureBreaksAProseRun() {
    let source = """
        Opening line.

        - a bullet

        After the list.

        | h |
        | --- |
        | c |

        After the table.

        ```
        code
        ```

        After the fence.

        > quoted

        ## A heading

        Under it.
        """
    let runs = Markdown.runs(Markdown.blocks(source))
    let shapes = runs.map { run -> String in
        switch run {
        case let .prose(paragraphs): "prose(\(paragraphs.count))"
        case let .block(block):
            switch block {
            case .bullet: "bullet"
            case .table: "table"
            case .code: "code"
            case .quote: "quote"
            case .heading: "heading"
            case .rule: "rule"
            case .numbered: "numbered"
            case .paragraph: "paragraph"
            }
        }
    }
    #expect(
        shapes == [
            "prose(1)", "bullet", "prose(1)", "table", "prose(1)", "code",
            "prose(1)", "quote", "heading", "prose(1)",
        ])
}

/// A run is never split by anything else, and a single paragraph is still a
/// run of one — the drawing of a lone paragraph must not change at all.
@Test func aLoneParagraphIsARunOfOne() {
    #expect(Markdown.runs(Markdown.blocks("Just this.")) == [.prose(["Just this."])])
    #expect(Markdown.runs([]) == [])
}

/// Inline syntax still parses per paragraph after the merge: the bold in the
/// second paragraph must not be swallowed by the first.
@Test func inlineSyntaxSurvivesTheMerge() {
    let merged = MarkdownText.merged(["Plain one.", "A **bold** word."])
    #expect(String(merged.characters) == "Plain one.\n\nA bold word.")
    let bolded = merged.runs.contains { run in
        run.inlinePresentationIntent?.contains(.stronglyEmphasized) == true
    }
    #expect(bolded)
}
