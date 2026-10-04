import Testing

@testable import AgentKit

#if canImport(AppKit)
import AppKit
#endif

/// Every phone empty state that explains something says it as a short lede and
/// rows, never a paragraph (ov-245). The Mac's `EmptyStateCopyTests` shape: a
/// lede of one short sentence, two or three rows of a real SF Symbol and at most
/// eight words each, and no layout talk. A board on an old runner is the one
/// exception, a single line short enough to need no rows.
@Suite struct PhoneEmptyStatesTests {
    static func expectScannable(_ copy: PhoneEmptyCopy, sourceLocation: SourceLocation = #_sourceLocation) {
        #expect(words(copy.lede) <= 12, "a lede of \(words(copy.lede)) words: \(copy.lede)", sourceLocation: sourceLocation)
        // One sentence: a full stop only at the end.
        #expect(!copy.lede.dropLast().contains("."), "more than one sentence: \(copy.lede)", sourceLocation: sourceLocation)
        if copy.rows.isEmpty { return }
        #expect((2...3).contains(copy.rows.count), "rows, not a paragraph", sourceLocation: sourceLocation)
        for row in copy.rows {
            #expect(words(row.text) <= 8, "a row of \(words(row.text)) words: \(row.text)", sourceLocation: sourceLocation)
            #expect(!row.text.hasSuffix("."), "a row is a list item: \(row.text)", sourceLocation: sourceLocation)
            #expect(row.text.first?.isUppercase == true, "sentence case: \(row.text)", sourceLocation: sourceLocation)
            #if canImport(AppKit)
            #expect(
                NSImage(systemSymbolName: row.symbol, accessibilityDescription: nil) != nil,
                "no SF Symbol named \(row.symbol)", sourceLocation: sourceLocation)
            #endif
        }
        let all = ([copy.lede] + copy.rows.map(\.text)).joined(separator: " ").lowercased()
        for layout in ["left", "right", "title bar", "sidebar", "below", "above"] {
            #expect(!all.contains(layout), "describes the layout: \(layout)", sourceLocation: sourceLocation)
        }
    }

    private static func words(_ text: String) -> Int {
        text.split(whereSeparator: \.isWhitespace).count
    }

    @Test func everyPhoneEmptyStateIsScannable() {
        for copy in PhoneEmptyStates.all { Self.expectScannable(copy) }
        #expect(PhoneEmptyStates.all.count == 7)
    }

    /// The Mac's own words, where the Mac has the same state, so a runner reads
    /// the same on every device.
    @Test func theStatesTheMacAlsoHasSayWhatTheMacSays() {
        #expect(PhoneEmptyStates.noOrchestrator.lede == "An orchestrator runs this workspace’s board.")
        #expect(PhoneEmptyStates.noOrchestrator.rows.map(\.text) == [
            "Tell it what you want done", "It plans tasks and puts agents on them",
            "It asks you when it needs a decision",
        ])
        #expect(PhoneEmptyStates.noWorktrees.lede == "A worktree is where an agent works.")
        #expect(PhoneEmptyStates.noRepositories.lede.hasPrefix("Add the repository"))
    }
}
