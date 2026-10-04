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
    /// The worktree an agent's pane works in, so a path its tool call names
    /// can be read as one of that worktree's files.
    @Entry var filesWorktree: Worktree? = nil
}

/// A path in an agent's tool call, as a link into Files when it's inside the
/// worktree the agent works in, else as plain text.
struct ToolLocationLabel: View {
    let location: String

    @Environment(\.openInFiles) private var openInFiles
    @Environment(\.filesWorktree) private var worktree

    var body: some View {
        if let openInFiles, let worktree, let path = FilesLogic.relative(location, in: worktree.path) {
            Button {
                openInFiles.open(worktree, path, nil)
            } label: {
                text
            }
            .buttonStyle(.link)
            .help("Show \(path) in Files")
        } else {
            text.foregroundStyle(.secondary)
        }
    }

    private var text: some View {
        Text(location)
            .font(.caption.monospaced())
            .lineLimit(1)
            .truncationMode(.middle)
    }
}
