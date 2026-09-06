// What a transcript row's body is allowed to redo while a reader scrolls.
//
// These are the guards for the scroll cost, and they assert on MISSES rather
// than on answers. A cache that has been disconnected from its caller still
// returns the right value for every input — it just returns it after doing all
// the work again, which is the exact defect being fixed. Only a miss count can
// tell the difference, so only a miss count is evidence.
//
// `.serialized` on the suites that touch a SHARED memo: those assert on
// process-wide state, and Swift Testing runs tests in parallel by default.
#if os(macOS) || os(iOS)
import Foundation
import SwiftUI
import Testing

@testable import AgentKit

@MainActor
@Suite struct RenderMemoTests {
    @Test func aRepeatedKeyIsComputedOnce() {
        let memo = RenderMemo<String, Int>(limit: 8)
        var calls = 0
        for _ in 0..<5 {
            _ = memo.value(for: "a") { _ in
                calls += 1
                return 1
            }
        }
        #expect(calls == 1)
        #expect(memo.misses == 1)
    }

    @Test func distinctKeysAreEachComputed() {
        let memo = RenderMemo<String, Int>(limit: 8)
        for key in ["a", "b", "c"] {
            _ = memo.value(for: key) { _ in key.count }
        }
        #expect(memo.misses == 3)
    }

    @Test func theAnswerIsTheSameOneTheComputationWouldHaveGiven() {
        let memo = RenderMemo<String, String>(limit: 8)
        let first = memo.value(for: "hello") { $0.uppercased() }
        let second = memo.value(for: "hello") { $0.uppercased() }
        #expect(first == "HELLO")
        #expect(second == "HELLO")
    }

    /// Bounded, because a transcript is not. Without eviction a long session
    /// pins every message it ever rendered plus every prefix the streaming tail
    /// row passed through.
    @Test func theOldestEntryIsEvictedPastTheLimit() {
        let memo = RenderMemo<Int, Int>(limit: 2)
        for key in [1, 2, 3] {
            _ = memo.value(for: key) { $0 }
        }
        // 1 was evicted by 3, so asking for it again is a fourth miss.
        _ = memo.value(for: 1) { $0 }
        #expect(memo.misses == 4)
        // 3 is still resident and costs nothing.
        _ = memo.value(for: 3) { $0 }
        #expect(memo.misses == 4)
    }

    /// Least-recently-USED, not least-recently-inserted. A reader paging back
    /// and forth over one screenful re-reaches the same rows, and evicting by
    /// insertion order would throw away exactly the ones being looked at.
    @Test func aHitKeepsAnEntryFromBeingTheNextEvicted() {
        let memo = RenderMemo<Int, Int>(limit: 2)
        _ = memo.value(for: 1) { $0 }
        _ = memo.value(for: 2) { $0 }
        _ = memo.value(for: 1) { $0 }  // 1 becomes the most recent
        _ = memo.value(for: 3) { $0 }  // evicts 2, not 1
        #expect(memo.misses == 3)
        _ = memo.value(for: 1) { $0 }
        #expect(memo.misses == 3, "the entry that was touched should have survived")
    }
}

@MainActor
@Suite(.serialized) struct MarkdownCacheTests {
    /// The wiring guard for the markdown parse.
    ///
    /// Rendering the same message twice must parse it once. Point
    /// `MarkdownText.body` back at `Markdown.runs(Markdown.blocks(text))` and
    /// this fails while every other markdown test in this package keeps
    /// passing — which is the whole reason it asserts on the miss count.
    @Test func renderingOneMessageTwiceParsesItOnce() {
        Markdown.runCache.removeAll()
        let text = """
            A paragraph with **bold** and `code`.

            - one
            - two

            Another paragraph entirely.
            """
        _ = renderedHeight(MarkdownText(text: text), width: 360)
        let afterFirst = Markdown.runCache.misses
        _ = renderedHeight(MarkdownText(text: text), width: 360)

        #expect(afterFirst == 1, "the first render should have parsed exactly once")
        #expect(
            Markdown.runCache.misses == 1,
            "a second render of the same message re-parsed it")
    }

    @Test func twoDifferentMessagesAreBothParsed() {
        Markdown.runCache.removeAll()
        _ = Markdown.cachedRuns("first message")
        _ = Markdown.cachedRuns("second message")
        #expect(Markdown.runCache.misses == 2)
    }

    /// The cache must not change what is drawn, only how often it is computed.
    @Test func theCachedRunsAreTheRunsTheParserProduces() {
        Markdown.runCache.removeAll()
        let text = "# Heading\n\nProse one.\n\nProse two.\n\n```\ncode\n```"
        let direct = Markdown.runs(Markdown.blocks(text))
        let cached = Markdown.cachedRuns(text)
        #expect(cached.count == direct.count)
        // `Run` is not Equatable, so compare the shape that matters: which runs
        // are prose and, for those, the paragraphs they carry.
        for (a, b) in zip(direct, cached) {
            switch (a, b) {
            case let (.prose(x), .prose(y)):
                #expect(x == y)
            case (.block, .block):
                break
            default:
                Issue.record("the cached runs have a different shape from the parsed ones")
            }
        }
    }
}

@MainActor
@Suite(.serialized) struct DiffCacheTests {
    /// The wiring guard for the diff.
    ///
    /// A collapsed diff row still computes its lines, because the header needs
    /// the count — so this is the cost a reader pays for scrolling past a large
    /// edit they never opened.
    @Test func theSameEditIsDiffedOnce() {
        DiffComputation.lineCache.removeAll()
        let old = (0..<200).map { "line \($0)" }.joined(separator: "\n")
        let new = old.replacingOccurrences(of: "line 100", with: "line one hundred")
        for _ in 0..<4 {
            _ = DiffComputation.cachedCompute(old: old, new: new)
        }
        #expect(DiffComputation.lineCache.misses == 1)
    }

    /// Keyed on the texts, so a second edit of one file is its own diff. A key
    /// that could not tell two edits apart would draw the first one's lines
    /// under the second one's header.
    @Test func aSecondEditOfTheSameFileIsItsOwnDiff() {
        DiffComputation.lineCache.removeAll()
        _ = DiffComputation.cachedCompute(old: "a\nb", new: "a\nc")
        _ = DiffComputation.cachedCompute(old: "a\nc", new: "a\nd")
        #expect(DiffComputation.lineCache.misses == 2)
    }

    @Test func theCachedLinesAreTheLinesTheDiffProduces() {
        DiffComputation.lineCache.removeAll()
        let old = "one\ntwo\nthree"
        let new = "one\ntwo and a half\nthree"
        let direct = DiffComputation.compute(old: old, new: new)
        let cached = DiffComputation.cachedCompute(old: old, new: new)
        #expect(direct.count == cached.count)
        for (a, b) in zip(direct, cached) {
            #expect(a.kind == b.kind)
            #expect(a.text == b.text)
            #expect(a.oldNumber == b.oldNumber)
            #expect(a.newNumber == b.newNumber)
        }
    }
}

/// The phone's `DiffView`, guarded by reading it.
///
/// Every other test in this file is about a memo, and a memo is exactly the
/// kind of thing that keeps returning the right answer after it stops being
/// used — so the call site is the half that needs a guard, and the Mac's is
/// rendered for real by `RenderWiringTests` in `apps/macos`. The phone has no
/// equivalent: `apps/ios` has a UI suite, CI COMPILES it and never runs it, so
/// nothing load-bearing can live there. Its copy of the one line that reaches
/// this memo — `DiffView.lines` in `AgentView.swift` — had nothing at all
/// holding it.
///
/// Reading the source is the second-best mechanism and it is chosen knowing
/// that: it pins a spelling rather than a behavior, and a determined revert
/// through a local alias would slip past it. What it does catch is the revert
/// that would actually happen — somebody deleting the word `cached` while
/// cleaning up — which is the failure this is written against.
///
/// `GeneratedFilesTest` on the Android side sets the precedent, including the
/// deliberate half of its trade: it fails loudly rather than skipping when the
/// file cannot be found, because a test that quietly passes when it checked
/// nothing is the same as no test.
@Suite struct PhoneDiffWiringTests {
    @Test func thePhonesDiffRowGoesThroughTheMemo() throws {
        let source = try #require(
            phoneAgentView(),
            """
            Could not find apps/ios/FarCooler/AgentView.swift by walking up from \
            this test. It is the only thing holding the phone's DiffView to the \
            diff memo; if the Swift has moved, point this at where it went rather \
            than deleting the check.
            """)
        let body = try #require(
            linesProperty(in: source),
            """
            DiffView.lines is no longer a `private var lines: [DiffComputation.Line]` \
            in AgentView.swift. If the shape changed, re-point this parse.
            """)

        #expect(
            body.contains("DiffComputation.cachedCompute"),
            """
            The phone's DiffView.lines does not go through \
            DiffComputation.cachedCompute. `body` reads `lines` even for a \
            COLLAPSED diff — the header and the "Show N lines" label both need the \
            count — so an uncached call is a 160,000-cell LCS on the main thread \
            for every diff row a reader scrolls past.
            """)
        #expect(
            !body.contains("DiffComputation.compute("),
            "The phone's DiffView.lines calls the uncached DiffComputation.compute.")
    }

    /// The phone's transcript view, found by walking up from this file.
    private func phoneAgentView() -> String? {
        var directory = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        while directory.path != "/" {
            let candidate = directory.appendingPathComponent("apps/ios/FarCooler/AgentView.swift")
            if let text = try? String(contentsOf: candidate, encoding: .utf8) { return text }
            directory = directory.deletingLastPathComponent()
        }
        return nil
    }

    /// The body of `DiffView`'s `lines`, and nobody else's.
    ///
    /// `AgentView.swift` is three thousand lines and the word `compute` could
    /// legitimately appear anywhere in it, so this narrows to the one
    /// declaration before it looks at anything.
    private func linesProperty(in source: String) -> String? {
        guard let view = source.range(of: "struct DiffView: View {") else { return nil }
        let rest = source[view.upperBound...]
        guard let declaration = rest.range(of: "private var lines: [DiffComputation.Line] {")
        else { return nil }
        let body = rest[declaration.upperBound...]
        guard let close = body.firstIndex(of: "}") else { return nil }
        return String(body[..<close])
    }
}
#endif
