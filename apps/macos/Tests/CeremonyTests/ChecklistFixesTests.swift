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
/// M3's): a plain tree, each level's text one step in from its parent's.
/// Read from what the rows lay out from (`SidebarGrid.chevron(depth:)`,
/// `text(depth:)`, `TerminalRow.columns(depth:)`, and the depth
/// `sidebarRows` gives each row), so a change to a row's inset, its chevron
/// column or a terminal's leading turns this red.
@MainActor
struct SidebarColumnTests {
    @Test func eachLevelsTextIsOneStepInFromItsParents() {
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
        // The worktree directly under its workspace.
        #expect(rows.map(\.kind) == [.repository, .workspace("Main"), .worktree("w")])
        let workspace = rows[1].depth, worktree = rows[2].depth
        // Repository text at 14, a workspace's chevron there too and its
        // name at 32; a worktree's chevron under that name, its title at 50.
        #expect(SidebarGrid.edge + SidebarGrid.indent(rows[0].depth) == 14)
        #expect(SidebarGrid.chevron(depth: workspace) == 14)
        #expect(SidebarGrid.text(depth: workspace) == 32)
        #expect(SidebarGrid.chevron(depth: worktree) == 32)
        #expect(SidebarGrid.text(depth: worktree) == 50)
        // A terminal's dot under its worktree's title, its name past it.
        let terminal = TerminalRow.columns(depth: worktree)
        #expect(terminal.glyph == 50)
        #expect(terminal.text == 65)
    }
}
