import AgentKit
import SwiftUI

/// The window's side of the jump bar and of Back and Forward (ov-192): what
/// a chosen item does, and the steps through history. Here rather than in
/// `ContentView.swift`, which is at its size ceiling.
struct JumpBarWindow: Equatable {
    /// Where the window has been, for ⌃⌘← and ⌃⌘→.
    var history = NavigationHistory()
    /// What the places in `history` were called when the window was last
    /// open, for naming them before their runners have answered (ov-248).
    var restoredTitles: [ContentView.Selection: String] = [:]
    /// Bumped by ⌘L: the jump bar takes the keyboard.
    var request = 0
    /// The jump bar has the keyboard: its Esc is its own, not Back.
    var active = false
}

extension ContentView {
    /// A jump bar item (ov-192): a place, by the one route an explicit open
    /// takes; a task's tab; a lost terminal's Restart or Dismiss, by ov-191's
    /// one mapping (`TerminalAction(_:)`).
    func jump(_ target: JumpTarget) {
        if case .lost(let pane, let action) = target, let ws = worktree(host: pane.host, id: pane.worktree),
            let term = ws.terminals.first(where: { $0.id == pane.terminal })
        {
            Task { await run(TerminalAction(action), on: term, in: ws) }
        }
        guard let next = target.place else { return }
        if next == trail { trail = nil }
        navigate(to: next)
        if let pick = target.taskTab { choose(pick.tab, for: pick.task) }
    }

    /// A place with the task it was opened from as the way back: a step
    /// through history, or a task's worktree from the jump bar (ov-185).
    func land(_ stop: NavigationHistory.Stop) {
        if let from = stop.trail, case .workspace(_, _, .worktree(let id, _)?) = stop.place {
            trail = from
            trailWorktree = id
        } else if stop.place == trail {
            trail = nil
        }
        navigate(to: stop.place)
    }

    /// The task `place` was opened from, while the window's trail holds it.
    func openedFrom(_ place: Selection?) -> Selection? {
        WorkspaceNavigation.keeps(trail: trail, opened: trailWorktree, now: place) ? trail : nil
    }

    /// Whether `place` is still somewhere to go, for a step through history.
    func resolves(_ place: Selection) -> Bool {
        NavigationHistory.resolves(
            place, in: store.fleet, repositories: { store.clients[$0]?.repositories.map(\.id) ?? [] },
            board: { boardStores["\($0)/\($1)"]?.board })
    }

    /// ⌃⌘← (`back`) and ⌃⌘→: where you were, or where Back left. Back in
    /// Focus leaves it first, and with no history goes up a level, as it
    /// did before there was one (`NavigationHistory.back`).
    func step(back: Bool) {
        let trail = openedFrom(selection)
        if back {
            switch jumpBar.history.back(focus: focusColumn, from: selection, trail: trail, resolves: resolves) {
            case .history(let stop): land(stop)
            case .upALevel: goBack()
            }
        } else if let stop = jumpBar.history.goForward(from: selection, trail: trail, resolves: resolves) {
            land(stop)
        }
    }

    /// The jump bar's crumbs' menus for `place`, built only when the bar
    /// asks (`JumpMenuSource`).
    func jumpMenus(
        _ crumbs: [WorkspaceNavigation.Crumb], place: Selection, host: String, summary: WorkspaceSummary?
    ) -> JumpMenuSource {
        JumpMenuSource(count: summary == nil ? 0 : crumbs.count) {
            JumpMenus.crumbs(
                crumbs, place: place, host: host, workspace: summary,
                board: summary.flatMap { w in store.clients[host].map { boardStore(for: w, client: $0, host: host).board } }
                    ?? .empty,
                fleet: store.fleet, groups: WorkspaceNumbers.groups(in: store.fleet), showsHosts: showHosts,
                waiting: { WorkspaceCounts.count(for: $0.workspace, host: $0.host, in: store.needsYou) },
                tab: taskTab(for: place), chosen: { chosenAgents[$0] })
        }
    }
}
