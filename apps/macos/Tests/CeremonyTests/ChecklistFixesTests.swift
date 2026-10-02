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

/// The sidebar's columns, measured from its edge (ov-83's grid, replacing
/// ov-78's and ov-63 M3's): a plain tree, each level one column in.
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
        // On ov-83's grid: repository text at A, a workspace's chevron
        // there too, its glyph at B and its name at C; a worktree's chevron
        // under that glyph, its glyph under the name, its title at D.
        // `GridGeometryTests` measures the drawn rows against the same.
        #expect(SidebarGrid.edge + SidebarGrid.indent(rows[0].depth) == ColumnGrid.a)
        #expect(SidebarGrid.chevron(depth: workspace) == ColumnGrid.a)
        #expect(SidebarGrid.glyph(depth: workspace) == ColumnGrid.b)
        #expect(SidebarGrid.text(depth: workspace) == ColumnGrid.c)
        #expect(SidebarGrid.chevron(depth: worktree) == ColumnGrid.b)
        #expect(SidebarGrid.text(depth: worktree) == ColumnGrid.d)
        // A terminal's dot under its worktree's title, its name a column on.
        let terminal = TerminalRow.columns(depth: worktree)
        #expect(terminal.glyph == ColumnGrid.d)
        #expect(terminal.text == ColumnGrid.column(4))
    }
}
