import Foundation
import Testing

@testable import Far_Cooler

/// Where the selection goes when the terminal it points at is gone.
///
/// ⌘W used to have a rule of its own, `selectNeighbour(of:)`, which walked the
/// whole fleet and took the first running terminal anywhere — on any runner —
/// while every other way a terminal disappears went through `healSelection`,
/// which stays in the worktree you were in. Closing a terminal now goes
/// through the one rule, and these pin what that rule does.
struct HealSelectionTests {
    private typealias Selection = ContentView.Selection

    private static func terminal(_ id: String, state: String = "running", exitCode: Int? = nil)
        -> Terminal
    {
        var t = Terminal(id: id, short: id, title: id, preset: "shell", state: state, epoch: 0)
        t.exitCode = exitCode
        return t
    }

    private static func workspace(_ id: String, host: String?, _ terminals: [Terminal])
        -> Workspace
    {
        Workspace(
            id: id, short: id, task: id, branch: "feat/\(id)", host: host,
            worktree: "/tmp/\(id)", state: "active", terminals: terminals)
    }

    /// The case `selectNeighbour` got wrong. The fleet lists another runner's
    /// running terminal FIRST, and the worktree the closed terminal was in
    /// still has one of its own that has exited cleanly — not running, not
    /// asking for anything. The selection stays in that worktree.
    @Test("A closed terminal's neighbour is in its own worktree, not another runner's")
    func aClosedTerminalsNeighbourIsInItsOwnWorktree() {
        let fleet = [
            Self.workspace("elsewhere", host: "gpu-box", [Self.terminal("busy")]),
            Self.workspace("here", host: nil, [Self.terminal("left", state: "exited", exitCode: 0)]),
        ]
        let closed: Selection = .terminal(host: "", workspace: "here", terminal: "gone")
        #expect(
            ContentView.healed(closed, in: fleet)
                == .terminal(host: "", workspace: "here", terminal: "left"))
    }

    /// Within the worktree, whatever wants you first, then whatever is running.
    @Test("Attention first, then running, within the worktree")
    func attentionFirstThenRunning() {
        let fleet = [
            Self.workspace(
                "here", host: nil,
                [
                    Self.terminal("idle-exit", state: "exited", exitCode: 0),
                    Self.terminal("running"),
                    Self.terminal("failed", state: "exited", exitCode: 1),
                ])
        ]
        let closed: Selection = .terminal(host: "", workspace: "here", terminal: "gone")
        #expect(
            ContentView.healed(closed, in: fleet)
                == .terminal(host: "", workspace: "here", terminal: "failed"))
    }

    /// The last terminal in a worktree lands on the worktree itself, not on a
    /// terminal somewhere else.
    @Test("The last terminal closed lands on its worktree")
    func theLastTerminalClosedLandsOnItsWorktree() {
        let fleet = [
            Self.workspace("elsewhere", host: "gpu-box", [Self.terminal("busy")]),
            Self.workspace("here", host: nil, []),
        ]
        let closed: Selection = .terminal(host: "", workspace: "here", terminal: "gone")
        #expect(ContentView.healed(closed, in: fleet) == .workspace(host: "", id: "here"))
    }

    /// A terminal that is still there is left selected. This is what keeps a
    /// close that FAILED from moving you anywhere.
    @Test("A terminal still in the fleet stays selected")
    func aTerminalStillThereStaysSelected() {
        let fleet = [Self.workspace("here", host: nil, [Self.terminal("a"), Self.terminal("b")])]
        let current: Selection = .terminal(host: "", workspace: "here", terminal: "b")
        #expect(ContentView.healed(current, in: fleet) == current)
    }
}
