import AgentKit
import SwiftUI

/// Where a path somebody points at opens: its worktree's Files (ov-189).
///
/// Set once, by the window (`ContentView`), and read by what names a path:
/// an agent's tool call, a file's heading in a diff. The window decides
/// where Files is for that worktree: a task's Files tab when the task on
/// screen works in it, else the Files inspector beside whatever is open.
struct OpenInFiles {
    /// Open `path`, relative to `worktree`'s root, at 1-based `line`.
    var open: (_ worktree: Worktree, _ path: String, _ line: Int?) -> Void
}

extension EnvironmentValues {
    @Entry var openInFiles: OpenInFiles? = nil
}
