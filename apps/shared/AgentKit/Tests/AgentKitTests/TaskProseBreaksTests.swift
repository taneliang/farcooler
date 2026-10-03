import Foundation
import Testing

@testable import AgentKit

/// `test/fixtures/task-prose-breaks.json`, which Android's `TaskProseBreaksTest`
/// reads too (ov-198): a single newline in a task's text is a line break inside
/// its paragraph, a blank line starts a new paragraph, and lists, code and
/// links come out as they always did.
private struct BreaksFixture: Decodable {
    struct Block: Decodable {
        var kind: String
        var text: String
        var level: Int?
        var number: String?
        var depth: Int?
        var language: String?

        var block: Markdown.Block? {
            switch kind {
            case "paragraph": .paragraph(text)
            case "heading": level.map { .heading(level: $0, text: text) }
            case "bullet": depth.map { .bullet(text: text, depth: $0) }
            case "numbered": number.flatMap { n in depth.map { .numbered(number: n, text: text, depth: $0) } }
            case "code": language.map { .code(text: text, language: $0) }
            default: nil
            }
        }
    }
    struct Case: Decodable {
        var `case`: String
        var text: String
        var blocks: [Block]
        var plain: [String]
    }
    var cases: [Case]

    static func load() throws -> BreaksFixture {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/task-prose-breaks.json"))
        return try JSONDecoder().decode(BreaksFixture.self, from: data)
    }
}

@MainActor
struct TaskProseBreaksTests {
    /// Each case's blocks, and each paragraph's words as drawn: the newline
    /// survives the inline pass too, so the `Text` breaks where the line did.
    @Test("A task's text breaks where it was broken, in the shared fixture")
    func theSharedFixturesBreaks() throws {
        let fixture = try BreaksFixture.load()
        #expect(fixture.cases.count >= 7)
        for each in fixture.cases {
            let expected = each.blocks.compactMap(\.block)
            #expect(expected.count == each.blocks.count, "\(each.case): every block kind is known")
            let blocks = TaskProse.blocks(each.text)
            #expect(blocks == expected, "\(each.case)")

            let paragraphs = blocks.compactMap { block -> String? in
                if case let .paragraph(text) = block { return text }
                return nil
            }
            #expect(paragraphs.map(TaskProse.plain) == each.plain, "\(each.case)")
        }
    }

    /// Adjacent paragraphs drawn as one `Text` (`MarkdownText.merged`) keep a
    /// line break as one newline and a paragraph as a blank line between.
    @Test("A drawn run keeps a break as a break and a paragraph as a blank line")
    func aDrawnRunKeepsBothKindsOfBreak() {
        let runs = Markdown.runs(TaskProse.blocks("One.\nTwo.\n\nThree."))
        guard case let .prose(paragraphs) = runs.first, runs.count == 1 else {
            Issue.record("expected one prose run, got \(runs)")
            return
        }
        #expect(String(MarkdownText.merged(paragraphs).characters) == "One.\nTwo.\n\nThree.")
    }
}
