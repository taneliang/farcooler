import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The ov-55 checklist's small Mac fixes, where they're values (ov-63).
@MainActor
struct ChecklistFixesTests {
    /// F3: Esc clears a search, then leaves the field.
    @Test func escapeInTheSearchFieldClearsThenLeaves() {
        #expect(SearchEscape.after(query: "bil") == ("", true))
        #expect(SearchEscape.after(query: "") == ("", false))
    }

    /// O7: a lost orchestrator is named no "shell".
    @Test func aLostOrchestratorsHeaderNamesNoShell() {
        let worktree = Worktree(
            id: "co", short: "co", task: "main", branch: "main", repository: "r", host: "",
            path: "/tmp/co", state: "active", terminals: [])
        func seat(_ preset: String, state: String) -> BoardPane {
            BoardPane(terminal: Terminal(id: "t", short: "t", title: "t", preset: preset, state: state, epoch: 0), worktree: worktree)
        }
        #expect(OrchestratorMenu.agentName(seat("claude", state: "running")) == "claude")
        #expect(OrchestratorMenu.agentName(seat("", state: "lost")) == nil)
        #expect(OrchestratorMenu.agentName(seat("zsh", state: "lost")) == nil)
        #expect(OrchestratorMenu.agentName(nil) == nil)
    }
}
