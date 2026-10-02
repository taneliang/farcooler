import Foundation
import Testing

@testable import Far_Cooler

/// ov-81 P14: the Changes pane reads a branch's base once and keeps saying it.
@MainActor
struct ChangesBaseOnceTests {
    private func set(branch: String = "feature", base: String, source: String? = nil) -> ChangeSet {
        ChangeSet(
            branch: branch, baseRef: base, baseSource: source, baseCommit: "", headCommit: "",
            insertions: 0, deletions: 0, commits: [], files: [], workingTree: nil)
    }

    @Test("A re-read with no base keeps the one the pane already had")
    func keepsTheBase() {
        let known = set(base: "main", source: "guessed")
        let kept = set(base: "").keepingBase(from: known)
        #expect(kept.baseRef == "main")
        #expect(kept.baseIsGuessed)
    }

    @Test("A base that came back is not overwritten, and another branch's is not borrowed")
    func onlyFillsAGap() {
        let known = set(base: "main")
        #expect(set(base: "develop").keepingBase(from: known).baseRef == "develop")
        #expect(set(branch: "other", base: "").keepingBase(from: known).baseRef == "")
    }

    @Test("An empty branch diff says its base once, with the caveat inside the one line")
    func oneLine() {
        #expect(ChangesPane.branchNothingDetail(baseRef: "", guessed: false)
            == "There’s nothing to compare this branch against yet.")
        #expect(ChangesPane.branchNothingDetail(baseRef: "main", guessed: false)
            == "This branch matches main.")
        let guessed = ChangesPane.branchNothingDetail(baseRef: "main", guessed: true)
        #expect(guessed.hasPrefix("This branch matches main, which was assumed"))
    }
}
