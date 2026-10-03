import AgentKit
import Foundation

// Task keys in the window's text as links (ov-196): which keys a runner's text
// may link, from the boards this window has read. The rule for which words are
// keys is AgentKit's `TaskKeyLinks`; this is only where the Mac's boards are.

extension TaskKeyIndex {
    /// `host`'s keys: its workspaces' prefixes and the tasks on the boards
    /// this window has read for it, the palette's tasks (`paletteTasks`).
    @MainActor
    static func mac(host: String, workspaces: [WorkspaceSummary], stores: some Sequence<TaskBoardStore>)
        -> TaskKeyIndex
    {
        var boards: [String: TaskBoardModel] = [:]
        for store in stores where store.client.target == host {
            boards[store.workspace.id] = store.board
        }
        return TaskKeyIndex(runner: host, workspaces: workspaces, boards: boards)
    }
}

extension TaskKeyLinker {
    /// The window's linker for `host`, each link opening its task through
    /// `openTask` (`ContentView.openTask(_:host:workspace:)`, the palette's
    /// way). Here rather than inline in `ContentView` so a test holds which
    /// of the target's ids goes where.
    @MainActor
    static func mac(
        host: String, workspaces: [WorkspaceSummary], stores: some Sequence<TaskBoardStore>,
        openTask: @escaping @MainActor (_ task: String, _ host: String, _ workspace: String) -> Void
    ) -> TaskKeyLinker {
        TaskKeyLinker(index: .mac(host: host, workspaces: workspaces, stores: stores)) {
            openTask($0.task, $0.runner, $0.workspace)
        }
    }
}
