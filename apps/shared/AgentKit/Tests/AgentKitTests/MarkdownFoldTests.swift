#if os(macOS) || os(iOS)
import SwiftUI
import Testing
@testable import AgentKit

/// A folded report draws only its start (ov-788): `Markdown.foldPrefix`.
/// It must fold exactly when the whole text folds, and show the same lines.
@MainActor
@Suite struct MarkdownFoldTests {
    static let lines = 4

    /// A lane's report as agents write them: headings, lists, a table, code
    /// and keys, repeated to about `kilobytes` KB.
    static func report(kilobytes: Int) -> String {
        var out = "## Lane ios-788: the transcript keeps its place (ov-788)\n\n**Commits**\n\n"
        var section = 0
        while out.utf8.count < kilobytes * 1_000 {
            section += 1
            out += "- `abc12\(section)` ios: a row above the screen is measured once (ov-\(600 + section))\n"
            out += "\n### Finding \(section)\n\n"
            out += "The fling measured **\(section) hitches**, the worst in `NativeAgentView.swift:\(section)`. "
                + "Every one lands when a row comes into view for the first time (ov-626, ov-675).\n\n"
            out += "| Frame | Self (ms) |\n|---|---:|\n| `TextLayout.measure` | 12 |\n| `FoldStack.plan` | 9 |\n\n"
            out += "```swift\nlet height = layout.measure(text)\nlet width = proposal.width ?? 320\n```\n\n"
            out += "- Point one: the row is three screens below the fold.\n  - Nested: it moves every row under it.\n"
        }
        return out + "\nThe last line of the report.\n"
    }

    /// Texts at the edges of the rule: short ones it must leave whole, and
    /// long ones of one kind of block each.
    static var texts: [String] {
        [
            "One line.",
            "Two paragraphs.\n\nThe second.",
            (1...6).map { "- Item \($0)" }.joined(separator: "\n"),
            (1...40).map { "- Item \($0)" }.joined(separator: "\n"),
            "```\n" + (1...200).map { "let x\($0) = \($0)" }.joined(separator: "\n") + "\n```\n\nAfter the code.",
            String(repeating: "A long paragraph of words that runs on and on. ", count: 300),
            "| A | B |\n|---|---|\n" + (1...60).map { "| \($0) | row \($0) |" }.joined(separator: "\n"),
            // The fold fills on a table's header: kept with its rule.
            (1...6).map { "Line \($0)" }.joined(separator: "\n") + "\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |",
            "# Heading\n\n---\n\n" + (1...30).map { "Line \($0)" }.joined(separator: "\n"),
            report(kilobytes: 12),
        ]
    }

    @Test func aShortTextIsDrawnWhole() {
        for text in Self.texts.prefix(3) {
            #expect(Markdown.foldPrefix(text, lines: Self.lines) == text)
        }
    }

    /// The cost the fold removes, and its gate: a report of any size draws
    /// a start of about the same size.
    @Test(arguments: [12, 100, 400])
    func aLongReportIsCutToItsStart(kilobytes: Int) {
        let text = Self.report(kilobytes: kilobytes)
        let prefix = Markdown.foldPrefix(text, lines: Self.lines)
        #expect(prefix.utf8.count <= 1_600, "a \(kilobytes) KB report drew \(prefix.utf8.count) bytes folded")
        #expect(text.hasPrefix(prefix))
    }

    /// Cut between source lines, the blocks before the cut are the whole
    /// text's own, and the cut one is the start of its own: a table kept
    /// with its rule, a fence read as code to the cut.
    @Test func theBlocksBeforeTheCutAreTheTextsOwn() {
        for text in Self.texts {
            let prefix = Markdown.foldPrefix(text, lines: Self.lines)
            let whole = Markdown.blocks(text)
            let cut = Markdown.blocks(prefix)
            #expect(Array(cut.dropLast()) == Array(whole.prefix(max(cut.count - 1, 0))), "\(text.prefix(40))")
            if let last = cut.last, let same = whole.dropFirst(cut.count - 1).first {
                // The cut block is the start of the whole text's own.
                switch (last, same) {
                case let (.paragraph(a), .paragraph(b)), let (.code(a, _), .code(b, _)):
                    #expect(b.hasPrefix(a), "the cut made a block that isn't the text's: \(a.suffix(40))")
                case let (.table(h1, r1), .table(h2, r2)):
                    #expect(h1 == h2 && Array(r2.prefix(r1.count)) == r1)
                default:
                    #expect(last == same, "the cut changed its last block from \(same) to \(last)")
                }
            }
        }
    }

    #if os(macOS)
    /// Drawn: it folds exactly when the whole text folds, and the fold
    /// shows the same pixels, at a phone's widths and a Mac's.
    @Test func itFoldsAndShowsTheSameAsTheWholeText() {
        let line = renderedHeight(Text(verbatim: "M").font(.body), width: 200)
        let cap = line * CGFloat(Self.lines + 1)
        for width in [240.0, 402.0, 680.0] {
            for text in Self.texts {
                let prefix = Markdown.foldPrefix(text, lines: Self.lines)
                let whole = renderedHeight(MarkdownText(text: text), width: width)
                let start = renderedHeight(MarkdownText(text: prefix), width: width)
                #expect((whole > cap + 1) == (start > cap + 1), "\(width): \(text.prefix(40)) folds \(whole) vs \(start)")
                guard prefix != text else { continue }
                let a = Self.fold(text, width: width, cap: cap), b = Self.fold(prefix, width: width, cap: cap)
                #expect(a == b, "\(width): the fold of \(text.prefix(40)) differs")
            }
        }
    }

    /// The top `cap` points of `text`, as pixels.
    static func fold(_ text: String, width: CGFloat, cap: CGFloat) -> Data? {
        let renderer = ImageRenderer(
            content: MarkdownText(text: text).fixedSize(horizontal: false, vertical: true)
                .frame(width: width, height: cap, alignment: .top).clipped())
        renderer.scale = 1
        guard let image = renderer.cgImage, let data = image.dataProvider?.data else { return nil }
        return data as Data
    }
    #endif
}
#endif
