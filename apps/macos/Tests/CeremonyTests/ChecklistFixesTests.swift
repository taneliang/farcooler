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

    /// F2: a narrow board says it short, on one line: a form that is
    /// shorter than the sentence and still carries the count.
    @Test func theWaitingPillHasAShortForm() {
        for count in [1, 2, 12] {
            let sentence = try! #require(TaskBoardModel.waitingSentence(count))
            let short = TaskBoardView.waitingShort(count)
            #expect(short.count < sentence.count, "the short form is no shorter: \(short)")
            #expect(short.hasPrefix("\(count) "))
        }
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

/// The sidebar's columns, measured from its edge (ov-78, replacing ov-63
/// M3's): 14 for a repository and a workspace's chevron; 32 for a workspace's
/// status glyph and its worktrees' chevrons, one step in; 50 for a
/// workspace's name, a worktree's title and, one step under that, its
/// terminals' status glyphs. A terminal's glyph is never in a chevron column.
@MainActor
struct SidebarColumnTests {
    @Test func aTerminalIsIndentedUnderItsWorktreesTitle() {
        let edge = SidebarGrid.edge, gutter = SidebarGrid.gutter
        #expect(edge == 14 && gutter == 18)
        let rows = ContentView.sidebarRows(
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
            open: { _ in true })
        // The worktree directly under its workspace: no Worktrees row.
        #expect(rows.map(\.kind) == [.repository, .workspace("Main"), .worktree("w")])
        let workspaceDepth = rows[1].depth, worktreeDepth = rows[2].depth
        #expect(workspaceDepth == 0 && worktreeDepth == 1)
        // A row's chevron is its first column; a workspace's glyph and a
        // worktree's title are one gutter past it.
        let workspaceGlyph = edge + CGFloat(workspaceDepth) * gutter + gutter
        let worktreeChevron = edge + CGFloat(worktreeDepth) * gutter
        let worktreeTitle = worktreeChevron + gutter
        #expect(worktreeChevron == workspaceGlyph && worktreeTitle == 50)
        let terminal = TerminalRow.columns(depth: worktreeDepth)
        #expect(terminal.glyph == worktreeTitle, "a terminal's glyph starts under its worktree's title")
        #expect(terminal.text > worktreeTitle, "a terminal's name is indented past its worktree's title")
    }
}
