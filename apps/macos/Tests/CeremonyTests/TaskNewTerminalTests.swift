import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// New Terminal on a task (ov-234): the task view's button.
@MainActor
struct TaskNewTerminalTests {
    private static let lane = Worktree(
        id: "w-7", short: "w7", task: "fix-it", branch: "fix-it", repository: "repo",
        host: "", path: "/tmp/fix-it", state: "active", terminals: [])

    /// Open Worktree runs first so Back returns to the task, then the shell
    /// is made in that task's own worktree, once.
    @Test("The task's New Terminal opens its worktree, then makes a shell in it")
    func opensTheWorktreeThenMakesAShell() throws {
        var events: [String] = []
        let action = TaskColumnModel.newTerminalAction(
            in: Self.lane, refused: false,
            openWorktree: { events.append("open") },
            newTerminal: { events.append("new:\($0.id)") })

        let run = try #require(action)
        run()

        #expect(events == ["open", "new:w-7"])
    }

    @Test("No button where the task has no worktree or its runner refuses")
    func noButtonWithoutAWorktreeOrAnswer() {
        #expect(
            TaskColumnModel.newTerminalAction(
                in: nil, refused: false, openWorktree: {}, newTerminal: { _ in }) == nil)
        #expect(
            TaskColumnModel.newTerminalAction(
                in: Self.lane, refused: true, openWorktree: {}, newTerminal: { _ in }) == nil)
    }
}
