import AgentKit
import Foundation

// Everything the sidebar does, and where else it's done (ov-86 review M1).
//
// A new window opens without the sidebar, so nothing may be reachable only
// from it. The title bar's switcher, the toolbar's runner item, the
// workspace navigator (ov-92: what the orchestrator's rail did is its first
// row) and the worktree menus are drawn from the values here, and
// `SidebarParityTests` checks that between them they offer every
// `SidebarAction`.

/// One thing the sidebar lets you do.
enum SidebarAction: String, CaseIterable {
    // The window's places.
    case needsYou, switchWorkspace, newWorkspace, newWorktree, addRepository, addRunner, find
    // The status bar: a runner in trouble, retried with a click, and a
    // runner whose Far Cooler is out of date.
    case runnerStatus, retryRunner, updateDaemon
    // A repository header's menu.
    case reconnect, newCheckoutTerminal, removeRepository
    // A worktree row and its menus.
    case openWorktree, showChanges, newTerminal, moveToWorkspace, useAsOrchestrator, hide, unhide, removeWorktree
}

/// What the workspace navigator offers on its own (ov-92): opening a loose
/// worktree from its row, its menus there, New Worktree… at the end of the
/// Worktrees section, and, with no orchestrator, Use as Orchestrator… on
/// the orchestrator's row. What its rows draw, from the same values.
extension Navigator {
    @MainActor
    static func offers(worktrees: BoardWorktrees, orchestrator: NavigatorOrchestrator?) -> Set<SidebarAction> {
        var out = Set<SidebarAction>()
        let rows = BoardWorktreesSection.rows(worktrees)
        if !rows.isEmpty { out.insert(.openWorktree) }
        for worktree in rows { out.formUnion(worktrees.menu(worktree).map(\.action)) }
        if !worktrees.hidden.isEmpty, worktrees.onUnhide != nil { out.insert(.unhide) }
        if worktrees.onNew != nil { out.insert(.newWorktree) }
        if let orchestrator, orchestrator.state == .none, !orchestrator.candidates.isEmpty,
            orchestrator.offers.contains(where: { if case .start = $0 { true } else { false } })
        {
            out.insert(.useAsOrchestrator)
        }
        return out
    }
}

/// What a worktree's menus offer away from the sidebar: on its row under
/// the navigator's Worktrees, on a task row's worktree, and in the
/// breadcrumb's worktree menu. The sidebar row's menus, one list.
enum WorktreeMenu {
    enum Item: Hashable {
        case open
        case showChanges
        case newTerminal
        /// Move to Workspace ▸, one item a workspace.
        case move(workspace: String, name: String)
        /// Use as Orchestrator, for one of its terminals.
        case useAsOrchestrator(terminal: String, label: String)
        case hide
        case unhide
        case remove
        /// Remove for a worktree whose directory is gone: Dismiss.
        case dismiss

        var action: SidebarAction {
            switch self {
            case .open: return .openWorktree
            case .showChanges: return .showChanges
            case .newTerminal: return .newTerminal
            case .move: return .moveToWorkspace
            case .useAsOrchestrator: return .useAsOrchestrator
            case .hide: return .hide
            case .unhide: return .unhide
            case .remove, .dismiss: return .removeWorktree
            }
        }

        var title: String {
            switch self {
            case .open: return "Open"
            case .showChanges: return "Show Changes"
            case .newTerminal: return "New Terminal"
            case .move(_, let name): return name
            case .useAsOrchestrator(_, let label): return "Use \(label) as Orchestrator"
            case .hide: return "Hide"
            case .unhide: return "Unhide"
            case .remove: return "Remove Worktree…"
            case .dismiss: return "Dismiss"
            }
        }
    }

    /// `worktree`'s items, in the sidebar's order. `showsChanges` is
    /// whether its runner reads changes; `moveTargets` are the workspaces a
    /// drag would move it to (`ContentView.moveTargets`); `adoptable` its
    /// terminals Use as Orchestrator is offered for. Nothing that writes
    /// while the runner is refused (`usable`), and no Hide or Remove for the
    /// main checkout.
    static func items(
        for worktree: Worktree, usable: Bool, showsChanges: Bool, moveTargets: [WorkspaceSummary],
        adoptable: [Terminal]
    ) -> [Item] {
        var out: [Item] = [.open]
        guard usable else { return out }
        if showsChanges { out.append(.showChanges) }
        out.append(.newTerminal)
        out += moveTargets.map { .move(workspace: $0.id, name: $0.name) }
        out += adoptable.map { .useAsOrchestrator(terminal: $0.id, label: $0.label) }
        if worktree.isHidden {
            out.append(.unhide)
        } else if !worktree.isMainCheckout {
            out.append(.hide)
        }
        if !worktree.isMainCheckout { out.append(worktree.worktreeMissing ? .dismiss : .remove) }
        return out
    }
}

/// What the title bar's switcher asks the window to do.
enum SwitcherCommand: Hashable {
    case go(ContentView.Selection)
    case needsYou
    case newWorkspace
    case newWorktree
    case addRepository
    case addRunner
    case runners
    case find
    case reconnect(host: String)
    case newCheckoutTerminal(host: String, repositoryID: String?, repository: String)
    case removeRepository(host: String, repositoryID: String?, repository: String)

    var action: SidebarAction {
        switch self {
        case .go: return .switchWorkspace
        case .needsYou: return .needsYou
        case .newWorkspace: return .newWorkspace
        case .newWorktree: return .newWorktree
        case .addRepository: return .addRepository
        case .addRunner: return .addRunner
        case .runners: return .runnerStatus
        case .find: return .find
        case .reconnect: return .reconnect
        case .newCheckoutTerminal: return .newCheckoutTerminal
        case .removeRepository: return .removeRepository
        }
    }
}

/// One line of the switcher's menu.
indirect enum SwitcherEntry: Hashable {
    /// A repository's name, over its workspaces.
    case header(String)
    case workspace(name: String, waiting: Int, current: Bool, number: Int?, command: SwitcherCommand)
    /// `badge`, when it isn't 0, is a count in the menu's own badge, at the
    /// item's trailing edge: never "Needs You (3)" (ov-101).
    case item(title: String, symbol: String, badge: Int = 0, command: SwitcherCommand)
    /// A line that says something and does nothing: the runners' state.
    case status(String, trouble: Bool)
    case submenu(title: String, symbol: String, entries: [SwitcherEntry])
    case separator

    /// Every command this line, or a menu under it, can send.
    var commands: [SwitcherCommand] {
        switch self {
        case .workspace(_, _, _, _, let command), .item(_, _, _, let command): return [command]
        case .submenu(_, _, let entries): return entries.flatMap(\.commands)
        case .header, .status, .separator: return []
        }
    }
}

enum WorkspaceSwitcherMenu {
    /// The switcher's menu: each repository's workspaces under its name,
    /// numbered; a menu of each repository's own actions (the sidebar
    /// header's: Reconnect, New Terminal in the checkout, Remove
    /// Repository…); then the runners' state, with Reconnect for each in
    /// trouble; then the places and the things to add.
    static func entries(
        groups: [WorkspaceNumbers.Group], current: (host: String, workspace: String)?,
        waiting: (WorkspaceNumbers.Place) -> Int, showsHosts: Bool, needsYou: Int, offersNewWorkspace: Bool,
        status: String, statusTrouble: Bool, troubled: [String]
    ) -> [SwitcherEntry] {
        var out: [SwitcherEntry] = []
        for group in groups {
            out.append(.header(showsHosts && !group.host.isEmpty ? "\(group.repository) · \(group.host)" : group.repository))
            for place in group.places {
                let here = current.map { $0.host == place.host && $0.workspace == place.workspace.id } ?? false
                out.append(
                    .workspace(
                        name: place.name, waiting: waiting(place), current: here, number: place.number,
                        command: .go(place.selection)))
            }
        }
        if !groups.isEmpty {
            out.append(.separator)
            out.append(
                .submenu(
                    title: "Repositories", symbol: "folder",
                    entries: groups.map { group in
                        .submenu(
                            title: group.repository, symbol: "folder",
                            entries: [
                                .item(title: "Reconnect", symbol: "arrow.clockwise", command: .reconnect(host: group.host)),
                                .item(
                                    title: "New Terminal in Checkout", symbol: "terminal",
                                    command: .newCheckoutTerminal(
                                        host: group.host, repositoryID: group.repositoryID, repository: group.repository)),
                                .separator,
                                .item(
                                    title: "Remove Repository…", symbol: "trash",
                                    command: .removeRepository(
                                        host: group.host, repositoryID: group.repositoryID, repository: group.repository)),
                            ])
                    }))
        }
        out.append(.separator)
        out.append(.status(status, trouble: statusTrouble))
        for host in troubled {
            out.append(
                .item(
                    title: "Reconnect \(host.isEmpty ? "This Mac" : host)", symbol: "arrow.clockwise",
                    command: .reconnect(host: host)))
        }
        out.append(.separator)
        out.append(.item(title: "Needs You", symbol: "tray", badge: needsYou, command: .needsYou))
        out.append(.item(title: "Go to Anything…", symbol: "magnifyingglass", command: .find))
        out.append(.separator)
        if offersNewWorkspace {
            out.append(.item(title: "New Workspace…", symbol: "plus.rectangle.on.rectangle", command: .newWorkspace))
        }
        out.append(.item(title: "New Worktree…", symbol: "plus", command: .newWorktree))
        out.append(.item(title: "Add Repository…", symbol: "folder.badge.plus", command: .addRepository))
        out.append(.item(title: "Add Device or Runner…", symbol: "qrcode", command: .addRunner))
        out.append(.item(title: "Runners and Devices…", symbol: "server.rack", command: .runners))
        return out
    }
}

/// The runners' trouble in the toolbar (ov-105, was ov-100's runner bar): a
/// small trailing item before Needs You, only while a runner is offline,
/// degraded, or behind this app's build. Nothing while every runner is
/// well and current; "N live" stays in the switcher's footer.
enum RunnerStatusItem {
    /// What is wrong with one runner.
    enum Problem: Equatable {
        /// Reconnecting, or unreachable.
        case offline
        /// Connected, and its tmux isn't answering.
        case noTmux
        /// Reachable, without Far Cooler.
        case notInstalled
    }

    struct Trouble: Equatable {
        let host: String
        let problem: Problem
    }

    /// One line of the item's menu.
    enum Entry: Hashable {
        /// Says something and does nothing.
        case note(String)
        case reconnect(host: String)
        case reconnectAll
        /// Opens the update card for the runners behind this app's build.
        case update(count: Int)
        case runners
        case separator

        var title: String {
            switch self {
            case .note(let text): return text
            case .reconnect(let host): return "Reconnect \(RunnerStatusItem.name(host))"
            case .reconnectAll: return "Reconnect All"
            case .update(let count): return count == 1 ? "Update Runner…" : "Update \(count) Runners…"
            case .runners: return "Runners and Devices…"
            case .separator: return ""
            }
        }

        /// The sidebar's action this line stands in for.
        var action: SidebarAction? {
            switch self {
            case .reconnect, .reconnectAll: return .retryRunner
            case .update: return .updateDaemon
            case .runners: return .runnerStatus
            case .note, .separator: return nil
            }
        }
    }

    static func name(_ host: String) -> String { host.isEmpty ? "This Mac" : host }

    /// What is wrong with a runner `FleetStore.unhealthyHosts` named, from
    /// its state; nil for one still connecting, which nothing is known of.
    static func problem(_ state: HostState) -> Problem? {
        switch state {
        case .connecting: return nil
        case .reconnecting, .unreachable: return .offline
        case .notInstalled: return .notInstalled
        case .connected: return .noTmux
        }
    }

    /// The item's words, or nil to show nothing at all.
    static func label(troubles: [Trouble], stale: [String]) -> String? {
        if troubles.count == 1, let one = troubles.first {
            let name = name(one.host)
            switch one.problem {
            case .offline: return "\(name) offline"
            case .noTmux: return "tmux unavailable on \(name)"
            case .notInstalled: return "Far Cooler isn’t on \(name)"
            }
        }
        if troubles.count > 1 {
            return troubles.allSatisfy { $0.problem == .offline }
                ? "\(troubles.count) runners offline" : "\(troubles.count) runners unavailable"
        }
        if !stale.isEmpty { return stale.count == 1 ? "Update available" : "\(stale.count) updates available" }
        return nil
    }

    /// The item's symbol: the warning while a runner is in trouble, the
    /// download arrow when the only news is an update.
    static func symbol(troubles: [Trouble]) -> String {
        troubles.isEmpty ? "arrow.down.circle" : "exclamationmark.triangle"
    }

    /// The item's menu: each runner in trouble and its Reconnect, Reconnect
    /// All when there's more than one, the update for runners behind this
    /// app's build, then Runners and Devices…. Empty while it's hidden.
    static func entries(troubles: [Trouble], stale: [String]) -> [Entry] {
        guard label(troubles: troubles, stale: stale) != nil else { return [] }
        var out: [Entry] = []
        if troubles.count > 1 {
            out += troubles.map { .note("\(name($0.host)): \(words($0.problem))") }
        }
        out += troubles.map { .reconnect(host: $0.host) }
        if troubles.count > 1 { out.append(.reconnectAll) }
        if !stale.isEmpty {
            if !out.isEmpty { out.append(.separator) }
            out.append(
                .note(
                    stale.count == 1
                        ? "\(name(stale[0])) isn’t running this app’s build"
                        : "\(stale.count) runners aren’t running this app’s build"))
            out.append(.update(count: stale.count))
        }
        out.append(.separator)
        out.append(.runners)
        return out
    }

    private static func words(_ problem: Problem) -> String {
        switch problem {
        case .offline: return "offline"
        case .noTmux: return "tmux unavailable"
        case .notInstalled: return "Far Cooler isn’t installed"
        }
    }
}
