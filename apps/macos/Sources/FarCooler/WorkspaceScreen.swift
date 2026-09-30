import AgentKit
import Foundation

// Which panes the detail puts on screen for a selection, column by column.
//
// A workspace shows up to two tmux layouts at once: its orchestrator's, in the
// conversation column, and a task's agent or an opened worktree in the third.
// Everything that asks "what is on screen" asks this, so the layout commands,
// ⌘] and ⌘[, "seen", and the drawing itself can't come to disagree about it:
// the one mistake worse than not marking a pane seen is marking one that isn't
// showing.

/// One tmux layout the detail draws, and where.
struct ShownLayout: Equatable {
    enum Column: Hashable {
        /// The workspace's orchestrator (spec §4.10).
        case conversation
        /// A task's agent (spec §4.4).
        case task
        /// A worktree opened whole: from the Worktrees disclosure, a task's
        /// Open Worktree, or a loose worktree.
        case worktree
    }

    var column: Column
    var worktree: Worktree
    var group: PaneGroup
    /// The layouts its bar offers: the worktree's own layouts for a worktree
    /// opened whole, and the one layout otherwise. A task's agent is shown as
    /// the one layout holding it, and an orchestrator's window is its own.
    var groups: [PaneGroup]

    var host: String { worktree.host ?? "" }

    func contains(_ pane: PaneRef) -> Bool {
        pane.host == host && pane.worktree == worktree.id && group.terminals.contains(pane.terminal)
    }
}

enum WorkspaceScreen {
    /// A workspace, as its runner lists it now: a listed one, or on a runner
    /// without workspaces the repository whose id it is. Nil once it's gone.
    static func workspace(_ id: String, host: String, in fleet: Fleet, repositories: [String]) -> WorkspaceSummary? {
        if let listed = fleet.runnerWorkspaces[host] { return listed.first { $0.id == id } }
        guard repositories.contains(id) else { return nil }
        return .implicit(repository: id)
    }

    /// Who `workspace`'s conversation column shows: the runner's live seat,
    /// `WorkspaceSummary.orchestrator`, first. Only when the runner names none
    /// is a terminal taken by its role, and then only a live one: a
    /// workspace can hold a stopped orchestrator beside the one that replaced
    /// it, and the column must never show the stopped one over it.
    static func orchestrator(of workspace: WorkspaceSummary, host: String, in fleet: Fleet) -> BoardPane? {
        guard !workspace.isImplicit else { return nil }
        if let seat = workspace.orchestrator, let pane = pane(seat, host: host, in: fleet) { return pane }
        for worktree in fleet.worktrees where (worktree.host ?? "") == host {
            if let live = worktree.terminals.first(where: {
                $0.isOrchestrator && $0.workspace == workspace.id
                    && [.running, .starting].contains(StateKind.parse($0.state))
            }) {
                return BoardPane(terminal: live, worktree: worktree)
            }
        }
        return nil
    }

    /// The panes working task `id` on `host`, in the runner's order: by
    /// `TaskAgentLink.isWorking`, the board's own rule, so the column and the
    /// card's Agent pill agree about the same task.
    static func agents(of id: String, host: String, in fleet: Fleet) -> [BoardPane] {
        fleet.worktrees.filter { ($0.host ?? "") == host }.flatMap { worktree in
            worktree.terminals
                // An orchestrator is its workspace's conversation, whatever
                // task id it carries: never a task's agent too (ov-63).
                .filter { !$0.isOrchestrator && TaskAgentLink.isWorking($0, on: id) }
                .map { BoardPane(terminal: $0, worktree: worktree) }
        }
    }

    /// The agent a task's column shows: the one chosen from its picker when
    /// that one is still working the task, else the first.
    static func agent(of id: String, host: String, in fleet: Fleet, chosen: String?) -> BoardPane? {
        let all = agents(of: id, host: host, in: fleet)
        return all.first { $0.terminal.id == chosen } ?? all.first
    }

    /// The terminals sharing `seat`'s tmux window: every one but the
    /// orchestrator itself (and any other orchestrator, which is never moved
    /// on its behalf). Empty before the layouts are read.
    ///
    /// The orchestrator is one pane (ov-78): its column never splits it, and
    /// a split or Show Changes made there opens in the main checkout
    /// instead. So anything in its window was put there some other way: a
    /// shell split beside a claude before it was adopted, a split made
    /// before ov-78, or a pane joined in with tmux itself. Until it's moved
    /// the column draws the window whole, with Move to Its Own Window for
    /// each of these; opening one from the main checkout moves it too, as
    /// does Use as Orchestrator. Nothing moves one unasked. ov-73 and ov-76
    /// left out the panes split there on purpose, by the runner's
    /// `split_of` and `split_of_orchestrator`; with nothing split there on
    /// purpose any more, every sharer is listed, and this app reads neither.
    static func sharers(of seat: BoardPane, layouts: [PaneGroup]?) -> [Terminal] {
        guard let group = layouts?.first(where: { $0.terminals.contains(seat.terminal.id) }) else { return [] }
        return group.terminals.compactMap { id in
            guard id != seat.terminal.id,
                let terminal = seat.worktree.terminals.first(where: { $0.id == id }),
                !terminal.isOrchestrator
            else { return nil }
            return terminal
        }
    }

    /// The seat whose window `terminal` shares in `worktree`, or nil when it
    /// shares none (or is a seat itself). A terminal in that window is drawn
    /// in the conversation column, not the checkout, until it's moved to its
    /// own window, which opening it from the checkout does first.
    static func seat(sharedBy terminal: String, in worktree: Worktree, fleet: Fleet, layouts: [PaneGroup]?) -> BoardPane? {
        seated(in: worktree, fleet: fleet).map(\.pane).first { seat in
            sharers(of: seat, layouts: layouts).contains { $0.id == terminal }
        }
    }

    /// The worktree the toolbar's Changes acts on for `selection` (ov-78):
    /// the worktree opened whole, in the third column or on its own, whether
    /// or not it has a terminal; with none opened, the main checkout the
    /// workspace's orchestrator runs in, whose changes open in the third
    /// column, never in the orchestrator's window. None with a task open,
    /// whose column shows its changes already (spec R3), or with no
    /// orchestrator seated.
    static func changesTarget(
        _ selection: ContentView.Selection?, in fleet: Fleet, repositories: [String] = []
    ) -> Worktree? {
        switch selection {
        case .looseWorktree(let host, let id, _), .workspace(let host, _, .worktree(let id, _)?):
            return WorkspaceSelection.worktree(host: host, id: id, in: fleet)
        case .workspace(let host, let id, nil):
            guard let workspace = workspace(id, host: host, in: fleet, repositories: repositories) else { return nil }
            return orchestrator(of: workspace, host: host, in: fleet)?.worktree
        default:
            return nil
        }
    }

    /// Whether `command`, with the keyboard in `key`, would add a pane to the
    /// orchestrator's window: a split or a new layout, with the key pane in
    /// the conversation column. Such a command opens a shell in the main
    /// checkout instead (ov-78: the orchestrator is one pane).
    static func opensShellInstead(_ command: TileCommand, key: PaneRef?, in shown: [ShownLayout]) -> Bool {
        guard [.splitRight, .splitDown, .newGroup].contains(command), let key else { return false }
        return shown.contains { $0.column == .conversation && $0.contains(key) }
    }

    /// Whether dropping `dragged` beside `target` would move a seated
    /// orchestrator, or put something in its window: `window` is the
    /// terminals in the window holding `target`. Refused (ov-78).
    static func joinsOrchestrator(
        _ dragged: String, window: [String], in worktree: Worktree, fleet: Fleet
    ) -> Bool {
        let seats = Set(seated(in: worktree, fleet: fleet).map(\.pane.terminal.id))
        return seats.contains(dragged) || window.contains(where: seats.contains)
    }

    /// The orchestrators seated in `worktree`, each with its workspace: the
    /// ones a conversation column draws. Every workspace's orchestrator runs
    /// in its repository's main checkout, so that's where these are.
    ///
    /// Seated, not every orchestrator-role terminal: a stopped one nobody
    /// seats is drawn among the checkout's own terminals, as the sidebar
    /// draws it (`sidebarRows`).
    static func seated(in worktree: Worktree, fleet: Fleet) -> [(workspace: WorkspaceSummary, pane: BoardPane)] {
        let host = worktree.host ?? ""
        return (fleet.runnerWorkspaces[host] ?? []).compactMap { workspace in
            guard let seat = orchestrator(of: workspace, host: host, in: fleet), seat.worktree.id == worktree.id
            else { return nil }
            return (workspace, seat)
        }
    }

    /// `worktree` without the orchestrators seated in it, which only their
    /// conversation columns draw. What opening it whole lists: its other
    /// terminals, a sharer of an orchestrator's window included, since
    /// opening that moves it to its own window first (ov-78).
    static func ownTerminals(of worktree: Worktree, fleet: Fleet) -> Worktree {
        worktree.without(Set(seated(in: worktree, fleet: fleet).map(\.pane.terminal.id)))
    }

    /// A terminal on `host`, with the worktree it's in.
    static func pane(_ terminal: String, host: String, in fleet: Fleet) -> BoardPane? {
        for worktree in fleet.worktrees where (worktree.host ?? "") == host {
            if let found = worktree.terminals.first(where: { $0.id == terminal }) {
                return BoardPane(terminal: found, worktree: worktree)
            }
        }
        return nil
    }

    /// Every layout the detail draws for `selection`, conversation first.
    ///
    /// `layouts` answers a worktree's layouts on its runner, nil before the
    /// first read; `chosen` is the agent picked in a task column's picker.
    static func shown(
        _ selection: ContentView.Selection?, in fleet: Fleet,
        layouts: (_ host: String, _ worktree: String) -> [PaneGroup]?,
        repositories: (_ host: String) -> [String] = { _ in [] },
        chosen: (_ task: String) -> String? = { _ in nil }
    ) -> [ShownLayout] {
        func holding(_ pane: BoardPane) -> PaneGroup? {
            (layouts(pane.worktree.host ?? "", pane.worktree.id) ?? []).first {
                $0.terminals.contains(pane.terminal.id)
            }
        }
        /// A worktree opened whole: the layout holding `terminal`, else its
        /// own active layout, with its own layouts in the bar.
        func opened(_ worktree: Worktree, terminal: String?) -> ShownLayout? {
            let all = layouts(worktree.host ?? "", worktree.id) ?? []
            let own = ContentView.ownLayouts(all, of: worktree)
            // An orchestrator's window is never a worktree's layout, even
            // named: it's the conversation column's. Nor is a window a
            // seated one shares with the terminal named: the column draws
            // that window whole until the terminal is moved to its own
            // (`sharers`), which opening it from the checkout does first. An
            // orchestrator nobody seats has no column, so the window it
            // shares is where the named terminal is shown.
            let orchestrators = ContentView.orchestrators(in: worktree)
            let seated = Set(seated(in: worktree, fleet: fleet).map(\.pane.terminal.id))
            if let terminal, !orchestrators.contains(terminal),
                let group = all.first(where: { $0.terminals.contains(terminal) }),
                !group.terminals.contains(where: seated.contains)
            {
                return ShownLayout(
                    column: .worktree, worktree: worktree, group: group,
                    groups: own.contains { $0.id == group.id } ? own : [group])
            }
            guard let group = ContentView.shownLayout(all, of: worktree), !group.terminals.isEmpty else {
                return nil
            }
            return ShownLayout(column: .worktree, worktree: worktree, group: group, groups: own)
        }

        switch selection {
        case nil, .needsYou:
            return []
        case .looseWorktree(let host, let id, let terminal):
            guard let worktree = WorkspaceSelection.worktree(host: host, id: id, in: fleet) else { return [] }
            return opened(worktree, terminal: terminal).map { [$0] } ?? []
        case .workspace(let host, let id, let focus):
            var out: [ShownLayout] = []
            if let workspace = workspace(id, host: host, in: fleet, repositories: repositories(host)),
                let seat = orchestrator(of: workspace, host: host, in: fleet),
                // Starting, the column draws "Starting Orchestrator…", not
                // the pane: nothing of it is on screen to be seen.
                StateKind.parse(seat.terminal.state) != .starting,
                let group = holding(seat)
            {
                out.append(ShownLayout(column: .conversation, worktree: seat.worktree, group: group, groups: [group]))
            }
            switch focus {
            case nil:
                break
            case .task(let task):
                if let agent = agent(of: task, host: host, in: fleet, chosen: chosen(task)),
                    let group = holding(agent)
                {
                    out.append(ShownLayout(column: .task, worktree: agent.worktree, group: group, groups: [group]))
                }
            case .worktree(let worktreeID, let terminal):
                if let worktree = WorkspaceSelection.worktree(host: host, id: worktreeID, in: fleet),
                    let shown = opened(worktree, terminal: terminal)
                {
                    out.append(shown)
                }
            }
            return out
        }
    }

    /// How many panes the layout holding `key` shows: what gates the
    /// prefix-less ⌃H ⌃J ⌃K ⌃L (`PrefixMode.tiledPanes`). The layout the
    /// keyboard is in, not whichever terminal view appeared last: beside a
    /// one-pane conversation, a task's three-pane agent layout traverses, and
    /// the other way round, ⌃L in the conversation still clears its screen.
    /// None with no key pane.
    static func tiledPanes(_ key: PaneRef?, in shown: [ShownLayout]) -> Int {
        guard let key, let layout = shown.first(where: { $0.contains(key) }) else { return 0 }
        return layout.group.panes.count
    }

    /// Of `shown`, the layouts a workspace actually draws in `arrangement`:
    /// the conversation only in a column of its own, and in the one-column
    /// form only while Orchestrator is picked; the third column only when
    /// it's drawn. Nothing while the detail hasn't been measured yet
    /// (`arrangement` nil).
    ///
    /// What "on screen" means for seen marks, the watching claim and the
    /// keyboard: a railed or hidden orchestrator marked seen would lose the
    /// notification it was about to send, for a pane nobody can see.
    static func visible(
        _ shown: [ShownLayout], arrangement: WorkspaceColumns.Arrangement?, pick: WorkspacePick
    ) -> [ShownLayout] {
        guard let arrangement else { return [] }
        return shown.filter { layout in
            switch layout.column {
            case .conversation:
                return arrangement.conversation == .column && (!arrangement.switcher || pick == .orchestrator)
            case .task, .worktree:
                return arrangement.task
            }
        }
    }

    /// What the selection becomes when the keyboard goes to `pane` (a click,
    /// ⌘], ⌃B o), or nil when the pane isn't on screen and has to be gone to
    /// (`WorkspaceSelection.landing`).
    ///
    /// A pane in the conversation leaves the selection as it is, even with
    /// the main checkout open whole beside it, where the orchestrator runs:
    /// naming it there would draw the orchestrator's window in the third
    /// column too. A pane in a worktree opened whole names it, so its row
    /// lights, staying in the workspace it was opened from.
    static func focusing(
        _ pane: PaneRef, selection: ContentView.Selection?, shown: [ShownLayout], fleet: Fleet
    ) -> ContentView.Selection?? {
        if shown.contains(where: { $0.column == .conversation && $0.contains(pane) }) { return .some(selection) }
        if let worktree = WorkspaceSelection.worktree(host: pane.host, id: pane.worktree, in: fleet),
            ContentView.shows(worktree, selection)
        {
            let next = ContentView.opening(worktree, terminal: pane.terminal, in: fleet)
            if case .workspace(let host, let id, .worktree(let wt, let t)) = next,
                case .workspace(_, let current, _)? = selection, current != id
            {
                return .some(.workspace(host: host, workspace: current, focus: .worktree(wt, terminal: t)))
            }
            return .some(next)
        }
        return shown.contains(where: { $0.contains(pane) }) ? .some(selection) : nil
    }

    /// The pane ⌥⌘1 or ⌥⌘3 gives the keyboard to in `layout`: the one tmux
    /// has focused, which is the one its view draws focused and hands typed
    /// keys to, else its first. Not simply its first: in a layout of two,
    /// typing would go to the second while ⌘W closed the first.
    static func columnPane(_ layout: ShownLayout) -> PaneRef? {
        (layout.group.focused ?? layout.group.terminals.first).map {
            PaneRef(host: layout.host, worktree: layout.worktree.id, terminal: $0)
        }
    }

    /// Whether a bare terminal, drawn before its layout is read, takes the
    /// keyboard: only with no tiled layout on screen to hold the key pane.
    static func bareTakesKeyboard(_ shown: [ShownLayout]) -> Bool { shown.isEmpty }

    /// Whether a terminal view drawing `layout` has the keyboard: it holds
    /// the key pane, and the board hasn't been given it (⌥⌘2). Only that
    /// view's focused pane draws focused and takes typed keys, so what you
    /// type, ⌃B and ⌘W all reach the same pane.
    static func hasKeyboard(_ layout: ShownLayout, key: PaneRef?, onBoard: Bool) -> Bool {
        guard !onBoard, let key else { return false }
        return layout.contains(key)
    }

    /// The pane the keyboard acts on: `key`, the pane last clicked or
    /// focused, while it's on screen; else the third column's focused pane,
    /// else the conversation's.
    ///
    /// Picked from what's shown so ⌃B, ⌘W and ⌘] act on a pane you can see.
    /// The third column leads when there is one, because opening a task or a
    /// worktree is going there.
    static func keyPane(_ key: PaneRef?, in shown: [ShownLayout], selection: ContentView.Selection?) -> PaneRef? {
        if let key, shown.contains(where: { $0.contains(key) }) { return key }
        guard let front = shown.last else { return nil }
        let named: String? = {
            switch selection {
            case .looseWorktree(_, _, let terminal): return terminal
            case .workspace(_, _, .worktree(_, let terminal)): return terminal
            default: return nil
            }
        }()
        let terminal =
            named.flatMap { front.group.terminals.contains($0) ? $0 : nil }
            ?? front.group.focused ?? front.group.terminals.first
        return terminal.map { PaneRef(host: front.host, worktree: front.worktree.id, terminal: $0) }
    }
}

extension ContentView {
    /// What the window's title says while `shown` is the layout in front.
    ///
    /// An orchestrator's pane is in the main checkout only because that's
    /// where the runner opens it, so it's titled with its workspace, as the
    /// board is. Anything else is titled with its worktree.
    static func frame(of shown: ShownLayout, in fleet: Fleet) -> (title: String, subtitle: String) {
        let worktree = shown.worktree
        if shown.column == .conversation,
            let terminal = shown.group.terminals.lazy.compactMap({ id in worktree.terminals.first { $0.id == id } })
                .first(where: \.isOrchestrator),
            let id = terminal.workspace,
            let workspace = fleet.runnerWorkspaces[shown.host]?.first(where: { $0.id == id })
        {
            let subtitle = [worktree.repository, "Orchestrator", shown.host.isEmpty ? nil : shown.host]
                .compactMap { $0 }
                .joined(separator: " · ")
            return (workspace.name, subtitle)
        }
        return (worktree.windowTitle, worktree.windowSubtitle)
    }
}

