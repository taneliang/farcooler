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

    private static func workspace(
        _ id: String, host: String?, repository: String? = nil, hidden: Bool = false,
        _ terminals: [Terminal]
    ) -> Workspace {
        Workspace(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: repository,
            host: host, worktree: "/tmp/\(id)", state: hidden ? "hidden" : "active",
            terminals: terminals)
    }

    /// The case the old ⌘W rule, `selectNeighbour`, got wrong. The fleet lists another runner's
    /// running terminal FIRST, and the worktree the closed terminal was in
    /// still has one of its own that has exited cleanly — not running, not
    /// asking for anything. The selection stays in that worktree.
    @Test("A closed terminal's neighbor is in its own worktree, not another runner's")
    func aClosedTerminalsNeighborIsInItsOwnWorktree() {
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

    // MARK: - The worktree itself is gone

    /// Another runner's worktree first in the merged fleet, as `FleetStore`
    /// appends runners, and two on this runner: one in another repository,
    /// one in the removed worktree's own.
    private static let afterRemoval = [
        workspace("theirs", host: "gpu-box", repository: "app", [terminal("busy")]),
        workspace("other-repo", host: nil, repository: "tools", []),
        workspace("same-repo", host: nil, repository: "app", []),
    ]
    private static let removed = workspace("gone", host: nil, repository: "app", [])

    /// It used to land on the first worktree in the merged fleet: here,
    /// another runner's.
    @Test("A removed worktree's terminal lands on a sibling in its repository, on its runner")
    func aRemovedWorktreesTerminalLandsOnASiblingOnItsRunner() {
        let selected: Selection = .terminal(host: "", workspace: "gone", terminal: "t")
        #expect(
            ContentView.healed(
                selected, in: Self.afterRemoval, was: Self.afterRemoval + [Self.removed])
                == .workspace(host: "", id: "same-repo"))
    }

    /// A selected worktree row was left selected after the worktree went.
    @Test("A removed worktree that was selected itself is healed too")
    func aSelectedWorktreeThatIsRemovedIsHealed() {
        let selected: Selection = .workspace(host: "", id: "gone")
        #expect(
            ContentView.healed(
                selected, in: Self.afterRemoval, was: Self.afterRemoval + [Self.removed])
                == .workspace(host: "", id: "same-repo"))
    }

    /// With the repository unknown (no previous fleet), any worktree on the
    /// runner the sidebar draws, before a hidden one.
    @Test("Without the old fleet, any shown worktree on the same runner")
    func withoutTheOldFleetAnyShownWorktreeOnTheSameRunner() {
        let fleet = [
            Self.workspace("theirs", host: "gpu-box", []),
            Self.workspace("tucked-away", host: nil, hidden: true, []),
            Self.workspace("shown", host: nil, []),
        ]
        #expect(
            ContentView.healed(.workspace(host: "", id: "gone"), in: fleet)
                == .workspace(host: "", id: "shown"))
    }

    /// Nothing left on the runner: nothing selected, rather than a jump to
    /// another runner.
    @Test("The last worktree on a runner lands on nothing, not on another runner")
    func theLastWorktreeOnARunnerLandsOnNothing() {
        let fleet = [Self.workspace("theirs", host: "gpu-box", [Self.terminal("busy")])]
        #expect(
            ContentView.healed(
                .terminal(host: "", workspace: "gone", terminal: "t"), in: fleet,
                was: fleet + [Self.removed]) == nil)
    }

    /// The Remove Worktree sheet heals as though the fleet had already dropped
    /// the worktree, and the fleet change heals when it lands. Either can run
    /// first, and the answer must not depend on which did.
    @Test("Removing a worktree lands in the same place whichever heal runs first")
    func removingAWorktreeLandsTheSameWhicheverHealRunsFirst() {
        let before = Self.afterRemoval + [Self.removed]
        let selected: Selection = .terminal(host: "", workspace: "gone", terminal: "t")
        // The sheet (was: [the removed worktree]) and then the fleet change.
        let sheetFirst = ContentView.healed(
            ContentView.healed(selected, in: Self.afterRemoval, was: [Self.removed]),
            in: Self.afterRemoval, was: before)
        // The fleet change, and then the sheet.
        let fleetFirst = ContentView.healed(
            ContentView.healed(selected, in: Self.afterRemoval, was: before),
            in: Self.afterRemoval, was: [Self.removed])
        #expect(sheetFirst == .workspace(host: "", id: "same-repo"))
        #expect(fleetFirst == sheetFirst)
    }
}
