import Foundation
import SwiftUI
import Testing

@testable import AgentKit

/// A task's own text, its intent, acceptance and notes, read as the Markdown
/// it's written in (ov-98): the same pieces on the Mac and the phone.
@MainActor
struct TaskProseTests {
    typealias Strike = AttributeScopes.SwiftUIAttributes.StrikethroughStyleAttribute

    /// Every element a ticket is written with comes out as itself: lines
    /// that break where they were broken, paragraphs, bullets, a numbered
    /// list, a fenced block, and inline bold, italic, code and links. Raw
    /// HTML stays the characters it was typed as.
    @Test("Each Markdown element in a task's text is drawn as itself")
    func eachElementIsDrawnAsItself() {
        let intent = """
            Owner, 2 Oct: the text is **hard** to read.
            A second line, kept.

            - one *item*
            - `two`

            1. first
            2. second

            ```
            let x = 1
            ```

            See [the spec](https://example.com/spec) and <b>raw</b>.
            """
        let blocks = TaskProse.blocks(intent)
        #expect(
            blocks == [
                .paragraph("Owner, 2 Oct: the text is **hard** to read.\nA second line, kept."),
                .bullet(text: "one *item*", depth: 0),
                .bullet(text: "`two`", depth: 0),
                .numbered(number: "1", text: "first", depth: 0),
                .numbered(number: "2", text: "second", depth: 0),
                .code(text: "let x = 1", language: ""),
                .paragraph("See [the spec](https://example.com/spec) and <b>raw</b>."),
            ])

        func style(of text: String, _ word: String) -> InlinePresentationIntent? {
            let line = TaskProse.inline(text)
            return line.runs.first { String(line[$0.range].characters) == word }?.inlinePresentationIntent
        }
        #expect(style(of: "the text is **hard** to read", "hard") == .stronglyEmphasized)
        #expect(style(of: "one *item*", "item") == .emphasized)
        #expect(style(of: "`two`", "two") == .code)

        let link = TaskProse.inline("See [the spec](https://example.com/spec).")
        #expect(link.runs.contains { $0.link == URL(string: "https://example.com/spec") })
        #expect(String(link.characters) == "See the spec.")

        let raw = TaskProse.inline("and <b>raw</b>.")
        #expect(String(raw.characters) == "and <b>raw</b>.", "HTML is drawn as the text it was typed as")
    }

    /// A met acceptance line is struck through, all of it, and keeps its
    /// own emphasis; one not met isn't struck at all.
    @Test("A met acceptance line is struck through, and keeps its Markdown")
    func aMetLineIsStruckThrough() {
        let met = TaskProse.acceptance("Renders **bold** and `code`", met: true)
        let open = TaskProse.acceptance("Renders **bold** and `code`", met: false)
        #expect(String(met.characters) == "Renders bold and code")
        #expect(met.runs.allSatisfy { $0[Strike.self] != nil }, "every run struck through")
        #expect(open.runs.allSatisfy { $0[Strike.self] == nil })
        #expect(met.runs.contains { $0.inlinePresentationIntent == .stronglyEmphasized })
        #expect(open.runs.contains { $0.inlinePresentationIntent == .code })
    }

    /// The quiet line over a note: its kind, who wrote it, and when.
    @Test("A note's line reads kind, byline and time")
    func aNotesLine() {
        #expect(TaskProse.noteLine(kind: "Finding", byline: "manager", ago: "4m ago") == "Finding · manager · 4m ago")
        #expect(TaskProse.noteLine(kind: "Answer", byline: "", ago: "now") == "Answer · now")
    }

    /// A task's text sits on the 8 pt rhythm (ov-83): every gap between two
    /// of its blocks is a whole number of steps. A reply keeps its own.
    @Test("A task's blocks are spaced on the 8 pt rhythm")
    func documentGapsAreOnTheRhythm() {
        let roles: [MarkdownBlockRole] = [
            .paragraph, .heading(level: 1), .heading(level: 2), .heading(level: 3), .listItem, .code, .quote,
            .rule, .table,
        ]
        for before in roles {
            for after in roles {
                let gap = MarkdownBlockSpacing.gap(after: before, before: after, style: .document)
                #expect(gap > 0 && gap.truncatingRemainder(dividingBy: 8) == 0, "\(before) → \(after): \(gap)")
                #expect(
                    MarkdownBlockSpacing.gap(after: before, before: after, style: .reply)
                        == MarkdownBlockSpacing.gap(after: before, before: after))
            }
        }
    }
}
