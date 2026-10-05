import Foundation

/// What revealing the orchestrator does (R-14). The sidebar has no
/// Orchestrator row; the chat column is the orchestrator, and ⌥⌘1 and the
/// title bar's Orchestrator item go to it: the plan peeked over it is put
/// away, and the keyboard goes in. Where the chat is a column beside the
/// canvas, what's open in the canvas stays; only where the chat is the main
/// area, and something else is in its place, does that go.
enum OrchestratorReveal {
    struct Step: Equatable {
        /// What's open is closed, so the chat can be the main area.
        var leavesWhatIsOpen: Bool
        /// The plan's peek over the chat is put away.
        var endsPeek: Bool
    }

    /// `conversation` as the window draws it now (nil before it has a
    /// width), and whether a task, worktree or plan page is open.
    static func step(conversation: WorkspaceColumns.Arrangement.Conversation?, somethingOpen: Bool) -> Step {
        Step(leavesWhatIsOpen: somethingOpen && conversation != .column, endsPeek: true)
    }
}
