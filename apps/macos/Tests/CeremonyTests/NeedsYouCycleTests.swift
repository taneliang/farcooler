import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Needs You on the Mac: hearing that the list moved, and walking it.
@MainActor
struct NeedsYouCycleTests {
    /// The CLI's `events` prints `needs_you_changed` as `{"kind":"needs_you"}`
    /// (`event_json` in crates/cli/src/main.rs). A line the stream decodes
    /// into nothing is a list the window never re-reads: an answered decision
    /// changes no terminal, so nothing else would bring it back.
    @Test("The needs_you event line re-reads the list")
    func theNeedsYouEventLineReReadsTheList() {
        var heard = 0
        EventStream.dispatch(Data(#"{"kind":"needs_you"}"#.utf8), decoder: JSONDecoder(), onNeedsYou: { heard += 1 })
        #expect(heard == 1)
        EventStream.dispatch(Data(#"{"kind":"fleet"}"#.utf8), decoder: JSONDecoder(), onNeedsYou: { heard += 1 })
        #expect(heard == 1, "a fleet line isn't news about the list")
    }

    /// `worktree list --json` carries each worktree's open tasks, and a
    /// pane with no task of its own is shown under its worktree's one open
    /// task: `TaskLink`'s rule, over this app's own types.
    @Test("A worktree's open tasks decode, and name a shell's task")
    func aWorktreesOpenTasksDecodeAndNameAShellsTask() throws {
        let json = #"""
            {"runtime_healthy":true,"live_panes":1,"worktrees":[{"id":"w-1","short":"w1","task":"fc-3-webhooks",
              "branch":"b","worktree":"/tmp/w","state":"active","open_tasks":[{"id":"t-9","key":"bil-9",
              "title":"Invoice PDF export","status":"in_progress"}],
              "terminals":[{"id":"s-1","short":"s1","title":"zsh","preset":"shell","state":"running","epoch":0}]}]}
            """#
        let fleet = try JSONDecoder().decode(Fleet.self, from: Data(json.utf8))
        let worktree = try #require(fleet.worktrees.first)
        #expect(worktree.openTasks?.map(\.key) == ["bil-9"])
        let shell = try #require(worktree.terminals.first)
        #expect(TaskLink.task(of: shell, in: worktree) == "t-9")
    }
}
