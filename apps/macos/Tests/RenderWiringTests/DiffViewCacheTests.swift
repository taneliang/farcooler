import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// That `DiffView` reaches the diff memo, rather than that the memo works.
///
/// `RenderMemoTests` in AgentKit already proves the memo: the same pair of
/// texts costs one `misses`, a second edit of the same file is its own entry,
/// and the cached lines are the lines the LCS produces. None of that is
/// evidence about this app. A cache nothing calls still answers every question
/// correctly — it just recomputes first, which IS the defect — so every one of
/// those tests stays green while `lines` is pointed back at
/// `DiffComputation.compute` and a reader pays a 160,000-cell LCS per collapsed
/// diff per scroll pass. The wiring was the unguarded half.
///
/// So this renders the real `DiffView`, through the same `ImageRenderer` shape
/// AgentKit's `renderedHeight` uses, and asserts on the memo's MISS count.
/// Point `lines` back at `compute` and the count stays at zero and this fails,
/// while every other test in this package and in AgentKit keeps passing.
///
/// ## Why not move the view into AgentKit
///
/// That was the suggested route, and it costs more than it buys. The two
/// `DiffView`s are deliberately not one view: the phone's scrolls
/// horizontally, carries a preference-key width so a run of added lines is one
/// band, spends 26 points on its gutter against the Mac's 34, and spells its
/// minus `-` where the Mac spells it U+2212. `AgentView.swift`'s own comment
/// states the rule — AgentKit holds the decode and the reduce, which both
/// platforms must agree on bit for bit, and layout is each platform's job.
/// Merging two layouts to make one testable would be changing what is drawn to
/// suit the test. This package's test targets already depend on the app target,
/// so the Mac's view can be rendered where it lives.
///
/// The phone's copy of that one line has no suite that CI executes at all; it
/// is guarded from `RenderMemoTests`, which reads it.
///
/// `.serialized` because `DiffComputation.lineCache` is process-wide state and
/// Swift Testing runs in parallel by default.
@MainActor
@Suite(.serialized) struct DiffViewCacheTests {
    /// Long enough to open COLLAPSED, which is the case that mattered: a
    /// collapsed diff draws none of its lines and still computes all of them,
    /// because the header and the "Show N lines" label both need the count.
    private var largeEdit: Diff {
        let old = (0..<200).map { "line \($0)" }.joined(separator: "\n")
        return Diff(
            path: "crates/daemon/src/file_diff.rs",
            oldText: old,
            newText: old.replacingOccurrences(of: "line 100", with: "line one hundred"))
    }

    @Test func drawingADiffRowReachesTheMemo() {
        DiffComputation.lineCache.removeAll()
        _ = renderedHeight(DiffView(diff: largeEdit), width: 620)
        #expect(
            DiffComputation.lineCache.misses == 1,
            "DiffView.lines is not going through DiffComputation.cachedCompute")
    }

    @Test func scrollingPastTheSameDiffTwiceComputesItOnce() {
        DiffComputation.lineCache.removeAll()
        let diff = largeEdit
        for _ in 0..<4 {
            _ = renderedHeight(DiffView(diff: diff), width: 620)
        }
        #expect(
            DiffComputation.lineCache.misses == 1,
            "a diff row re-diffed its edit on a later body evaluation")
    }

    /// The memo must not change what is drawn, only how often it is computed —
    /// so the collapsed row still says how many lines it is hiding.
    @Test func theCollapsedRowStillKnowsItsLineCount() {
        DiffComputation.lineCache.removeAll()
        let diff = largeEdit
        let drawn = renderedHeight(DiffView(diff: diff), width: 620)
        #expect(drawn > 0)
        #expect(DiffComputation.cachedCompute(old: diff.oldText ?? "", new: diff.newText).count > 20)
    }
}

/// Offscreen, at a fixed width, the way `MarkdownLayoutTests` does it.
///
/// A copy rather than a shared helper because AgentKit's is internal to its own
/// test target; four lines of `ImageRenderer` is a smaller thing to keep honest
/// than a test-support product neither package has.
@MainActor
func renderedHeight<Content: View>(_ content: Content, width: CGFloat) -> CGFloat {
    let renderer = ImageRenderer(content: content.frame(width: width))
    renderer.proposedSize = ProposedViewSize(width: width, height: nil)
    renderer.scale = 1
    return renderer.cgImage.map { CGFloat($0.height) } ?? 0
}
