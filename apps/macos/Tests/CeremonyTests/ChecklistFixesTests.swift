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

    /// F2: a narrow board says it short, on one line.
    @Test func theWaitingPillHasAShortForm() {
        #expect(TaskBoardView.waitingShort(2) == "2 waiting")
    }

    /// O7: a lost orchestrator's header names no "shell".
    @Test func aLostOrchestratorsHeaderNamesNoShell() {
        let worktree = Worktree(
            id: "co", short: "co", task: "main", branch: "main", repository: "r", host: "",
            path: "/tmp/co", state: "active", terminals: [])
        func seat(_ preset: String, state: String) -> BoardPane {
            BoardPane(terminal: Terminal(id: "t", short: "t", title: "t", preset: preset, state: state, epoch: 0), worktree: worktree)
        }
        #expect(ConversationHeader.agentName(seat("claude", state: "running")) == "claude")
        #expect(ConversationHeader.agentName(seat("", state: "lost")) == nil)
        #expect(ConversationHeader.agentName(seat("zsh", state: "lost")) == nil)
        #expect(ConversationHeader.agentName(nil) == nil)
    }
}

/// The sidebar's text columns, measured from its edge (ov-63 M3): 14 for a
/// repository and a workspace's glyph, 32 for a workspace's name and a
/// worktree's chevron, 50 for a worktree's title, and nothing deeper.
@MainActor
struct SidebarColumnTests {
    @Test func aTerminalTakesNoIndentStepOfItsOwn() {
        let edge = SidebarGrid.edge, gutter = SidebarGrid.gutter
        #expect(edge == 14 && gutter == 18)
        let worktreeDepth = ContentView.sidebarRows(
            fleet: {
                var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [
                    Worktree(
                        id: "w", short: "w", task: "w", branch: "b", repository: "r", host: "", path: "/tmp/w",
                        state: "active", terminals: [], repositoryID: "repo", workspace: "ws")
                ], branchPrefix: nil)
                fleet.runnerWorkspaces[""] = [
                    WorkspaceSummary(id: "ws", name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: "repo")
                ]
                return fleet
            }(),
            open: { _ in true }
        ).first { if case .worktree = $0.kind { true } else { false } }!.depth
        let worktreeTitle = edge + CGFloat(worktreeDepth) * gutter + gutter
        #expect(worktreeTitle == 50)
        let terminal = TerminalRow.columns(depth: worktreeDepth)
        #expect(terminal.glyph == 32, "a terminal's glyph sits in its worktree's chevron column")
        #expect(terminal.text <= worktreeTitle, "a terminal's name starts deeper than its worktree's title")
    }
}
