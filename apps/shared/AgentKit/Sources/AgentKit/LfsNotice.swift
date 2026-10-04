import Foundation

// A worktree says when large files weren't downloaded (ov-199).
//
// A worktree of a Git LFS repository hydrates its large files best effort when
// the runner makes it: an object missing from the runner's LFS store, or a
// hydration that ran out of time, leaves the pointer file. The runner counts
// them (`Worktree.lfs_pointers`) and the Changes screen of the worktree says so,
// with a way to try again (`worktree.hydrate_lfs`). Android's `LfsNotice.kt`
// holds the same decisions in Kotlin; the words differ only by each platform's
// own capitalization.

/// What the Changes screen says about a worktree's pointer files, or nothing.
public struct LfsNotice: Equatable, Sendable {
    /// How many large files are still pointers. Always above zero.
    public var pointers: Int
    /// Whether this device may ask the runner to try again: not on the `read`
    /// scope, which sees the sentence without the button.
    public var canRetry: Bool

    public init(pointers: Int, canRetry: Bool) {
        self.pointers = pointers
        self.canRetry = canRetry
    }

    /// The notice for a worktree reporting `pointers`, or nil when it reports
    /// none. A runner that predates the count sends nothing, which reads as no
    /// notice rather than as zero files being fine.
    public static func make(pointers: Int?, mayAct: Bool) -> LfsNotice? {
        guard let pointers, pointers > 0 else { return nil }
        return LfsNotice(pointers: pointers, canRetry: mayAct)
    }

    public static let title = "Some large files weren’t downloaded."
    /// What Try Again can do and what it can't. The runner never downloads:
    /// it fills in files whose objects are already in the repository's LFS
    /// store on the runner. `git lfs pull` run in this worktree fetches the
    /// objects of the worktree's own commit, whatever branch it's on, which
    /// a pull in the main checkout would not.
    public static let detail =
        "Try Again fills in any that are already on this runner. To download the rest, run “git lfs pull” in this worktree."
    /// Said in place of the detail when the runner couldn’t be asked.
    public static let unreachable = "Couldn’t ask the runner to try again. Check that it’s connected."
    public static let retry = "Try Again"
    /// Not "Downloading…": the runner only copies from its own LFS store.
    public static let retrying = "Trying Again…"
}
