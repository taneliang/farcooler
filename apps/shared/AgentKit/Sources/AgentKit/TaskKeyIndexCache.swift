import Foundation

/// A runner's `TaskKeyIndex`, built once per change in its boards, and the
/// same value, its storage and all, while the keys it links are the same
/// (ov-791).
///
/// Every view under a screen reads the linker from the environment, and
/// SwiftUI compares the new one with the old on each pass to know whether
/// to draw it again. An index built fresh on each pass is a dictionary of
/// every task on the runner's boards, so each of those comparisons walked
/// all of them: on a board of 800 tasks moving once a second, about 15 ms
/// of the main thread per move, a dropped frame while flinging a task's
/// screen. An index handed back from here compares by its storage, at once.
@MainActor
public final class TaskKeyIndexCache {
    private var inputs: Inputs?
    private var built: TaskKeyIndex = .empty
    /// How many times it has made an index that differs from the last, for
    /// a test.
    public private(set) var builds = 0

    private struct Inputs: Equatable {
        var runner: String
        var workspaces: [WorkspaceSummary]
        var version: Int?
        var boards: [String: TaskBoardModel]
    }

    public init() {}

    /// The index for these reads: the last one built while they're the
    /// same, and while what they link is the same, though a task on them
    /// moved.
    ///
    /// `version`, where the caller keeps one, is a number that moves
    /// whenever the boards do, compared in their place: a board read again
    /// is a new dictionary, which compared deeply on every pass walked
    /// every task until it next moved.
    public func index(
        runner: String, workspaces: [WorkspaceSummary], boards: [String: TaskBoardModel], version: Int? = nil
    ) -> TaskKeyIndex {
        let next = Inputs(runner: runner, workspaces: workspaces, version: version, boards: version == nil ? boards : [:])
        if next == inputs { return built }
        inputs = next
        let made = TaskKeyIndex(runner: runner, workspaces: workspaces, boards: boards)
        if made != built {
            built = made
            builds += 1
        }
        return built
    }
}
