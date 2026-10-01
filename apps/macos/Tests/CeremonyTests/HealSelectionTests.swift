import AgentKit
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

    private static func worktree(
        _ id: String, host: String?, repository: String? = nil, hidden: Bool = false,
        _ terminals: [Terminal]
    ) -> Worktree {
        Worktree(
            id: id, short: id, task: id, branch: "feat/\(id)", repository: repository,
            host: host, path: "/tmp/\(id)", state: hidden ? "hidden" : "active",
            terminals: terminals)
    }

    /// The case the old ⌘W rule, `selectNeighbour`, got wrong. The fleet lists another runner's
    /// running terminal FIRST, and the worktree the closed terminal was in
    /// still has one of its own that has exited cleanly — not running, not
    /// asking for anything. The selection stays in that worktree.
    @Test("A closed terminal's neighbor is in its own worktree, not another runner's")
    func aClosedTerminalsNeighborIsInItsOwnWorktree() {
        let fleet = [
            Self.worktree("elsewhere", host: "gpu-box", [Self.terminal("busy")]),
            Self.worktree("here", host: nil, [Self.terminal("left", state: "exited", exitCode: 0)]),
        ]
        let closed: Selection = .looseWorktree(host: "", worktree: "here", terminal: "gone")
        #expect(
            ContentView.healed(closed, in: fleet)
                == .looseWorktree(host: "", worktree: "here", terminal: "left"))
    }

    /// Within the worktree, whatever wants you first, then whatever is running.
    @Test("Attention first, then running, within the worktree")
    func attentionFirstThenRunning() {
        let fleet = [
            Self.worktree(
                "here", host: nil,
                [
                    Self.terminal("idle-exit", state: "exited", exitCode: 0),
                    Self.terminal("running"),
                    Self.terminal("failed", state: "exited", exitCode: 1),
                ])
        ]
        let closed: Selection = .looseWorktree(host: "", worktree: "here", terminal: "gone")
        #expect(
            ContentView.healed(closed, in: fleet)
                == .looseWorktree(host: "", worktree: "here", terminal: "failed"))
    }

    /// The last terminal in a worktree lands on the worktree itself, not on a
    /// terminal somewhere else.
    @Test("The last terminal closed lands on its worktree")
    func theLastTerminalClosedLandsOnItsWorktree() {
        let fleet = [
            Self.worktree("elsewhere", host: "gpu-box", [Self.terminal("busy")]),
            Self.worktree("here", host: nil, []),
        ]
        let closed: Selection = .looseWorktree(host: "", worktree: "here", terminal: "gone")
        #expect(ContentView.healed(closed, in: fleet) == .looseWorktree(host: "", worktree: "here", terminal: nil))
    }

    /// A terminal that is still there is left selected. This is what keeps a
    /// close that FAILED from moving you anywhere.
    @Test("A terminal still in the fleet stays selected")
    func aTerminalStillThereStaysSelected() {
        let fleet = [Self.worktree("here", host: nil, [Self.terminal("a"), Self.terminal("b")])]
        let current: Selection = .looseWorktree(host: "", worktree: "here", terminal: "b")
        #expect(ContentView.healed(current, in: fleet) == current)
    }

    // MARK: - The worktree itself is gone

    /// Another runner's worktree first in the merged fleet, as `FleetStore`
    /// appends runners, and two on this runner: one in another repository,
    /// one in the removed worktree's own.
    private static let afterRemoval = [
        worktree("theirs", host: "gpu-box", repository: "app", [terminal("busy")]),
        worktree("other-repo", host: nil, repository: "tools", []),
        worktree("same-repo", host: nil, repository: "app", []),
    ]
    private static let removed = worktree("gone", host: nil, repository: "app", [])

    /// It used to land on the first worktree in the merged fleet: here,
    /// another runner's.
    @Test("A removed worktree's terminal lands on a sibling in its repository, on its runner")
    func aRemovedWorktreesTerminalLandsOnASiblingOnItsRunner() {
        let selected: Selection = .looseWorktree(host: "", worktree: "gone", terminal: "t")
        #expect(
            ContentView.healed(
                selected, in: Self.afterRemoval, was: Self.afterRemoval + [Self.removed])
                == .looseWorktree(host: "", worktree: "same-repo", terminal: nil))
    }

    /// A selected worktree row was left selected after the worktree went.
    @Test("A removed worktree that was selected itself is healed too")
    func aSelectedWorktreeThatIsRemovedIsHealed() {
        let selected: Selection = .looseWorktree(host: "", worktree: "gone", terminal: nil)
        #expect(
            ContentView.healed(
                selected, in: Self.afterRemoval, was: Self.afterRemoval + [Self.removed])
                == .looseWorktree(host: "", worktree: "same-repo", terminal: nil))
    }

    /// With the repository unknown (no previous fleet), any worktree on the
    /// runner the sidebar draws, before a hidden one.
    @Test("Without the old fleet, any shown worktree on the same runner")
    func withoutTheOldFleetAnyShownWorktreeOnTheSameRunner() {
        let fleet = [
            Self.worktree("theirs", host: "gpu-box", []),
            Self.worktree("tucked-away", host: nil, hidden: true, []),
            Self.worktree("shown", host: nil, []),
        ]
        #expect(
            ContentView.healed(.looseWorktree(host: "", worktree: "gone", terminal: nil), in: fleet)
                == .looseWorktree(host: "", worktree: "shown", terminal: nil))
    }

    /// Nothing left on the runner: nothing selected, rather than a jump to
    /// another runner.
    @Test("The last worktree on a runner lands on nothing, not on another runner")
    func theLastWorktreeOnARunnerLandsOnNothing() {
        let fleet = [Self.worktree("theirs", host: "gpu-box", [Self.terminal("busy")])]
        #expect(
            ContentView.healed(
                .looseWorktree(host: "", worktree: "gone", terminal: "t"), in: fleet,
                was: fleet + [Self.removed]) == nil)
    }

    /// The Remove Worktree sheet heals as though the fleet had already dropped
    /// the worktree, and the fleet change heals when it lands. Either can run
    /// first, and the answer must not depend on which did.
    @Test("Removing a worktree lands in the same place whichever heal runs first")
    func removingAWorktreeLandsTheSameWhicheverHealRunsFirst() {
        let before = Self.afterRemoval + [Self.removed]
        let selected: Selection = .looseWorktree(host: "", worktree: "gone", terminal: "t")
        // The sheet (was: [the removed worktree]) and then the fleet change.
        let sheetFirst = ContentView.healed(
            ContentView.healed(selected, in: Self.afterRemoval, was: [Self.removed]),
            in: Self.afterRemoval, was: before)
        // The fleet change, and then the sheet.
        let fleetFirst = ContentView.healed(
            ContentView.healed(selected, in: Self.afterRemoval, was: before),
            in: Self.afterRemoval, was: [Self.removed])
        #expect(sheetFirst == .looseWorktree(host: "", worktree: "same-repo", terminal: nil))
        #expect(fleetFirst == sheetFirst)
    }

    // MARK: - A worktree drilled into

    /// A worktree drilled into in a workspace stays in that workspace when a
    /// terminal in it closes, and goes back up to the workspace when the
    /// worktree itself goes. A workspace with
    /// nothing open, or a task, has nothing to heal.
    @Test("A workspace's opened worktree heals within the workspace")
    func aWorkspacesOpenedWorktreeHealsWithinTheWorkspace() {
        let fleet = [Self.worktree("here", host: nil, [Self.terminal("left")])]
        let closed: Selection = .workspace(host: "", workspace: "ws", focus: .worktree("here", terminal: "gone"))
        #expect(
            ContentView.healed(closed, in: fleet)
                == .workspace(host: "", workspace: "ws", focus: .worktree("here", terminal: "left")))
        let removed: Selection = .workspace(host: "", workspace: "ws", focus: .worktree("gone", terminal: "t"))
        #expect(ContentView.healed(removed, in: fleet) == .workspace(host: "", workspace: "ws", focus: nil))
        let task: Selection = .workspace(host: "", workspace: "ws", focus: .task("t-9"))
        #expect(ContentView.healed(task, in: fleet) == task)
        #expect(ContentView.healed(.needsYou, in: fleet) == .needsYou)
    }

    /// A loose worktree removed lands on a sibling as its row would open it:
    /// under the workspace that owns it, never as a loose worktree a
    /// workspace claims.
    @Test("A removed loose worktree lands on a claimed sibling under its owner")
    func aRemovedLooseWorktreeLandsOnAClaimedSiblingUnderItsOwner() {
        var claimed = Self.worktree("same-repo", host: nil, repository: "app", [])
        claimed.workspace = "ws-main"
        claimed.repositoryID = "r-app"
        let workspaces = ["": [WorkspaceSummary(id: "ws-main", name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: "r-app")]]
        #expect(
            ContentView.healed(
                .looseWorktree(host: "", worktree: "gone", terminal: nil), in: [claimed],
                was: [claimed, Self.removed], workspaces: workspaces)
                == .workspace(host: "", workspace: "ws-main", focus: .worktree("same-repo", terminal: nil)))
    }
}
