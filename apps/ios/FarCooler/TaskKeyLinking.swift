import SwiftUI

// Task keys in a runner's text as links (ov-196): which keys may link, from
// the boards this connection has read, and opening one as a pane's task chip
// does, by pushing the task. The rule for which words are keys is AgentKit's
// `TaskKeyLinks`.

extension Connection {
    /// What "ov-190" in this runner's text links to, and where it goes:
    /// the task, pushed over the screen showing. Links nothing without a
    /// navigator to push with, or before the runner is known.
    func taskKeyLinker(_ navigator: PhoneNavigator?) -> TaskKeyLinker {
        guard let runner = hostId?.uuidString, let navigator else { return .none }
        let index = TaskKeyIndex(runner: runner, workspaces: fleet.workspaces ?? [], boards: boards)
        return TaskKeyLinker(index: index) { navigator.open($0.phoneRoute) }
    }
}
