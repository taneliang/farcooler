import AppKit
import Testing

@testable import Far_Cooler

/// Every empty state that explains something says it as a short lede and
/// rows, never a paragraph (ov-205). The owner on the paragraph under "No
/// Workspace Selected": "too many words. illustrations or bullets instead
/// so that it's more scannable?"
@MainActor
struct EmptyStateCopyTests {
    /// The shape that keeps an empty state scannable: a lede of one short
    /// sentence, two or three rows of a real SF Symbol and at most eight
    /// words each, and no layout talk.
    static func expectScannable(_ copy: EmptyStateCopy, sourceLocation: SourceLocation = #_sourceLocation) {
        #expect((2...3).contains(copy.rows.count), "rows, not a paragraph", sourceLocation: sourceLocation)
        if let lede = copy.lede {
            #expect(words(lede) <= 12, "a lede of \(words(lede)) words: \(lede)", sourceLocation: sourceLocation)
            // One sentence: a full stop only at the end.
            #expect(!lede.dropLast().contains("."), "more than one sentence: \(lede)", sourceLocation: sourceLocation)
        }
        for row in copy.rows {
            #expect(words(row.text) <= 8, "a row of \(words(row.text)) words: \(row.text)", sourceLocation: sourceLocation)
            #expect(!row.text.hasSuffix("."), "a row is a list item: \(row.text)", sourceLocation: sourceLocation)
            #expect(row.text.first?.isUppercase == true, "sentence case: \(row.text)", sourceLocation: sourceLocation)
            #expect(
                NSImage(systemSymbolName: row.symbol, accessibilityDescription: nil) != nil,
                "no SF Symbol named \(row.symbol)", sourceLocation: sourceLocation)
        }
        let all = ([copy.lede].compactMap { $0 } + copy.rows.map(\.text)).joined(separator: " ").lowercased()
        for layout in ["left", "right", "title bar", "sidebar", "below", "above"] {
            #expect(!all.contains(layout), "describes the layout: \(layout)", sourceLocation: sourceLocation)
        }
    }

    private static func words(_ text: String) -> Int {
        text.split(whereSeparator: \.isWhitespace).count
    }

    @Test("No Repositories and No Worktrees say what a worktree does in rows")
    func fleetStates() {
        Self.expectScannable(FleetPlaceholder.noRepositoriesCopy)
        Self.expectScannable(FleetPlaceholder.noWorktreesCopy)
        #expect(FleetPlaceholder.noRepositoriesCopy.lede?.hasPrefix("Add the repository") == true)
        #expect(FleetPlaceholder.noWorktreesCopy.lede?.hasPrefix("A worktree is where an agent works") == true)
    }

    @Test("Nothing Needs You lists what waits there, one row each")
    func needsYou() {
        Self.expectScannable(NeedsYouView.emptyCopy)
        #expect(NeedsYouView.emptyCopy.rows.count == 3)
    }

    @Test("No Orchestrator says what one is for, then what it does, in rows")
    func noOrchestrator() {
        Self.expectScannable(ConversationColumn.emptyCopy)
        #expect(ConversationColumn.emptyCopy.lede?.hasPrefix("An orchestrator runs") == true)
    }
}
