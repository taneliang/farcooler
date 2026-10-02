import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Nothing the sidebar does is reachable only from it (ov-86 review M1). A
/// new window opens without the sidebar, so every `SidebarAction` has to be
/// offered by the title bar's switcher, the runner banner or a worktree's
/// menus, which are drawn from the values asked of here.
@MainActor
struct SidebarParityTests {
    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let main = WorkspaceSummary(
        id: "ws-main", name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: repo)
    private static let billing = WorkspaceSummary(
        id: "ws-bil", name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: repo)

    private static func worktree(_ id: String, state: String = "active", terminals: [Terminal] = []) -> Worktree {
        Worktree(
            id: id, short: id, task: id, branch: id, repository: "shop", host: "", path: "/tmp/\(id)", state: state,
            terminals: terminals, repositoryID: repo, workspace: main.id)
    }

    /// Every action, from the most each surface offers: a fleet with a
    /// workspace, a runner in trouble and out of date, a worktree with a
    /// terminal to adopt and somewhere to move to, and a hidden one.
    private static func offered() -> Set<SidebarAction> {
        let shell = Terminal(id: "t1", short: "t1", title: "claude", preset: "claude", state: "running", epoch: 0)
        var fleet = Fleet(
            runtimeHealthy: true, livePanes: 0, worktrees: [worktree("lane", terminals: [shell])], branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [main, billing]
        let switcher = WorkspaceSwitcherMenu.entries(
            groups: WorkspaceNumbers.groups(in: fleet), current: nil, waiting: { _ in 0 }, showsHosts: false,
            needsYou: 1, offersNewWorkspace: true, status: "tmux unavailable", statusTrouble: true, troubled: ["box"])
        let banner = RunnerBanner.actions(trouble: true, unhealthy: ["box"], stale: ["box"])
        let lane = WorktreeMenu.items(
            for: worktree("lane", terminals: [shell]), usable: true, showsChanges: true, moveTargets: [billing],
            adoptable: [shell])
        let hidden = WorktreeMenu.items(
            for: worktree("old", state: "hidden"), usable: true, showsChanges: true, moveTargets: [], adoptable: [])
        return Set(switcher.flatMap(\.commands).map(\.action) + banner + (lane + hidden).map(\.action))
    }

    @Test("Every sidebar action has a way in without the sidebar")
    func everyActionHasAnotherWayIn() {
        let missing = SidebarAction.allCases.filter { !Self.offered().contains($0) }
        #expect(missing.isEmpty, "only in the sidebar: \(missing)")
    }

    /// The main checkout is never hidden or removed from a menu, and a
    /// refused runner offers nothing that writes, as the sidebar row.
    @Test("A worktree's menu keeps the sidebar row's rules")
    func worktreeMenuRules() {
        var checkout = Self.worktree("checkout")
        checkout.is_main_checkout = true
        let items = WorktreeMenu.items(
            for: checkout, usable: true, showsChanges: true, moveTargets: [], adoptable: [])
        #expect(!items.contains(.hide) && !items.contains(.remove))
        #expect(WorktreeMenu.items(
            for: Self.worktree("lane"), usable: false, showsChanges: true, moveTargets: [Self.billing], adoptable: []
        ) == [.open])
    }

    /// Each repository in the switcher carries its header's actions.
    @Test("Each repository in the switcher has Reconnect, New Terminal and Remove Repository")
    func repositoriesCarryTheirActions() {
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [Self.worktree("lane")], branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [Self.main]
        let entries = WorkspaceSwitcherMenu.entries(
            groups: WorkspaceNumbers.groups(in: fleet), current: nil, waiting: { _ in 0 }, showsHosts: false,
            needsYou: 0, offersNewWorkspace: false, status: "1 live", statusTrouble: false, troubled: [])
        let repositories = entries.compactMap { entry -> [SwitcherEntry]? in
            if case .submenu("Repositories", _, let inner) = entry { return inner }
            return nil
        }.first ?? []
        #expect(repositories.count == 1)
        #expect(Set(repositories.flatMap(\.commands).map(\.action)) == [.reconnect, .newCheckoutTerminal, .removeRepository])
    }
}
