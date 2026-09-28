import AgentKit
import Foundation

// The sidebar's shape: repository, then workspace, then that workspace's
// board, orchestrator and worktrees, with the worktrees no workspace owns in
// one collapsed Unclaimed group per repository.
//
// Worked out here, as values, and drawn by `ContentView.sidebar` from them.
// Which workspace owns which worktree, and in what order workspaces come, is
// AgentKit's rule (`WorkspaceGrouping`), shared with the phones; what this file
// adds is this app's half — the runner each row is on, the display name a
// header shows, the orchestrator pulled out of the worktree it runs in, and
// the hidden worktrees kept at the bottom as before.

/// One row of the sidebar, before it is drawn.
struct SidebarEntry: Identifiable {
    enum Kind: Equatable {
        /// A repository's header — or a silent runner's, with no project.
        case repository
        /// A workspace's header, by its name. Drawn even when Main is the
        /// only one, so the model is visible before the first split.
        case workspace(String)
        case board
        /// The workspace's orchestrator, or the row saying it has none.
        case orchestrator
        /// A worktree, by its id.
        case worktree(String)
        /// The repository's worktrees no workspace owns, collapsed.
        case unclaimed(count: Int)
        /// The repository's hidden worktrees, collapsed.
        case hidden(count: Int)
    }

    let kind: Kind
    let host: String
    /// The repository's display name: what its header says. Empty for a
    /// silent runner's header, and "Ungrouped" for rows with no repository.
    let project: String
    /// The repository's uuid, or nil from a CLI too old to send one.
    let repositoryID: String?
    /// The workspace a workspace, board or orchestrator row is for. An
    /// implicit one on a runner without workspaces.
    var workspace: WorkspaceSummary?
    /// The worktree a worktree row draws, without its orchestrators.
    var worktree: Worktree?
    /// The orchestrator, and the worktree it runs in, which is where
    /// selecting it goes. Nil on an orchestrator row with none running; on a
    /// workspace header, the same seat, for its menu.
    var orchestrator: BoardPane?
    /// A repository header's shown worktrees, or an Unclaimed or Hidden
    /// group's.
    var worktrees: [Worktree] = []
    /// How many steps in from the repository's header this row is drawn:
    /// 1 for everything under a repository that has workspaces, so the
    /// workspace level reads as one; 0 for the layout from before them.
    var depth = 0

    /// Which repository this row is under, on which runner: by uuid where the
    /// CLI sends one, so two repositories that share a name are two groups.
    var group: String { "\(host)\u{1}\(repositoryID ?? project)" }

    /// What a repository's collapsed state and its Hidden group are
    /// remembered by. The display name, as before workspaces, so nobody's
    /// sidebar reopens differently after an update.
    var collapseKey: String { "\(host)\u{1}\(project)" }

    var id: String {
        switch kind {
        case .repository: return group
        case .workspace: return "\(group)\u{1}workspace\u{1}\(workspace?.id ?? "")"
        case .board: return "\(group)\u{1}board\u{1}\(workspace?.id ?? "")"
        case .orchestrator: return "\(group)\u{1}orchestrator\u{1}\(workspace?.id ?? "")"
        case .worktree(let id): return "\(group)\u{1}worktree\u{1}\(id)"
        case .unclaimed: return "\(group)\u{1}unclaimed"
        case .hidden: return "\(group)\u{1}hidden"
        }
    }
}

extension ContentView {
    /// The sidebar's rows, in order, for `fleet` — every runner's worktrees
    /// and every runner's workspaces.
    ///
    /// - Grouped by runner, then repository: two runners can have a
    ///   project of the same name, and they are not the same project. The
    ///   host is only displayed when there is more than one runner.
    /// - Worktrees matching `query` only, and orchestrators matching it. A
    ///   worktree is matched as its row draws it, without the orchestrators
    ///   drawn in their own rows: typing an orchestrator's name finds its
    ///   row, not the checkout it runs in. While searching, a workspace with
    ///   no match is left out, and so is a repository: a header with nothing
    ///   under it would look like a hit.
    /// - A runner without workspaces (`fleet.runnerWorkspaces` has no key for
    ///   it) keeps the layout from before them: repository, board, worktrees.
    /// - `silentHosts` are runners that have contributed nothing yet; each
    ///   gets a header, as before.
    static func sidebarRows(
        fleet: Fleet, query: String = "", silentHosts: [String] = []
    ) -> [SidebarEntry] {
        struct Group {
            let host: String
            let project: String
            let repositoryID: String?
            var worktrees: [Worktree] = []
        }
        var order: [String] = []
        var groups: [String: Group] = [:]
        for worktree in fleet.worktrees {
            let host = worktree.host ?? ""
            let project = (worktree.repository ?? "").isEmpty ? "Ungrouped" : worktree.repository!
            let key = "\(host)\u{1}\(worktree.repositoryID ?? project)"
            if groups[key] == nil {
                order.append(key)
                groups[key] = Group(host: host, project: project, repositoryID: worktree.repositoryID)
            }
            groups[key]?.worktrees.append(worktree)
        }

        var rows: [SidebarEntry] = []
        for key in order {
            guard let group = groups[key] else { continue }
            let entries = repositoryRows(group.host, group.project, group.repositoryID, group.worktrees, fleet, query)
            // Only a header: nothing in this repository matched the search.
            if entries.count > 1 { rows += entries }
        }
        // A runner that has never connected has no rows of its own, and
        // without this it would simply be missing — leaving you to wonder
        // where it went rather than seeing that it needs attention. Skipped
        // while searching: a runner with nothing on it can never match a
        // query, and a header appearing only here would look like a hit.
        if query.isEmpty {
            for host in silentHosts {
                rows.append(SidebarEntry(kind: .repository, host: host, project: "", repositoryID: nil))
            }
        }
        return rows
    }

    /// Every terminal in `rows`, in the order they are drawn: an
    /// orchestrator in its own row under its workspace, each worktree's
    /// terminals in turn, then Unclaimed's and Hidden's, collapsed or not.
    ///
    /// What ⌘] and ⌘[, ⌥⌘↓ and ⌥⌘↑, ⌘1… and the attention cycle walk, so
    /// stepping goes down the list a user can see rather than jumping about
    /// in the runner's order, which the grouping no longer draws.
    static func terminalOrder(_ rows: [SidebarEntry]) -> [Terminal] {
        var seen: Set<String> = []
        var out: [Terminal] = []
        func add(_ terminals: [Terminal]) {
            for terminal in terminals where seen.insert(terminal.id).inserted { out.append(terminal) }
        }
        for row in rows {
            switch row.kind {
            case .orchestrator: add(row.orchestrator.map { [$0.terminal] } ?? [])
            case .worktree: add(row.worktree?.terminals ?? [])
            case .unclaimed, .hidden: add(row.worktrees.flatMap(\.terminals))
            case .repository, .workspace, .board: continue
            }
        }
        return out
    }

    /// One repository's rows: its header, each workspace's, then Unclaimed
    /// and Hidden.
    private static func repositoryRows(
        _ host: String, _ project: String, _ repositoryID: String?, _ all: [Worktree],
        _ fleet: Fleet, _ query: String
    ) -> [SidebarEntry] {
        // The runner's order, and nothing else. This used to partition the
        // main checkout to the top, which was stable and was still the app
        // deciding: with rows draggable, a rule here silently outranks the
        // one the person dragging just expressed, and the card they moved
        // springs back with nothing to explain why. The runner's rank starts
        // out as exactly what that partition produced — main checkout first,
        // then by worktree path — see migration 0009.
        func entry(_ kind: SidebarEntry.Kind) -> SidebarEntry {
            SidebarEntry(kind: kind, host: host, project: project, repositoryID: repositoryID)
        }
        let listed = fleet.runnerWorkspaces[host]
        let workspaces =
            project == "Ungrouped"
            ? [] : (listed ?? []).filter { $0.repository != nil && $0.repository == repositoryID }

        // The orchestrators drawn in their own rows, by terminal id: these,
        // and only these, are left out of the worktree rows, so each is drawn
        // exactly once — and an orchestrator-role terminal no row shows (a
        // stopped one the runner no longer seats) stays where it is. Found
        // before the search, which matches each worktree as it is drawn.
        var seats: [String: BoardPane] = [:]
        for workspace in workspaces where !workspace.isImplicit {
            seats[workspace.id] = Self.orchestrator(of: workspace, host: host, in: fleet)
        }
        let drawn = Set(seats.values.map(\.terminal.id))
        let matched = all.filter { $0.without(drawn).matches(query) }
        let shown = matched.filter { !$0.isHidden }
        let hidden = matched.filter(\.isHidden)

        var header = entry(.repository)
        header.worktrees = shown
        var rows = [header]
        var body: [SidebarEntry] = []
        var unclaimedIDs: [String] = []
        let byID = Dictionary(shown.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })

        if project == "Ungrouped" {
            // No repository, so no board and no workspaces: the rows alone.
            for worktree in shown { body.append(entry(.worktree(worktree.id))) }
        } else {
            let grouped = WorkspaceGrouping.group(
                repository: repositoryID ?? "", workspaces: workspaces,
                worktrees: shown.map { (id: $0.id, workspace: $0.workspace) },
                orchestrators: [:])

            for group in grouped.workspaces {
                let workspace = group.workspace
                let seat = seats[workspace.id]
                let seatMatches = seat.map { Self.matches($0.terminal, query) } ?? false
                if !query.isEmpty && group.worktrees.isEmpty && !seatMatches { continue }
                if !workspace.isImplicit {
                    var title = entry(.workspace(workspace.name))
                    title.workspace = workspace
                    // For its menu, which offers Replace rather than Start
                    // while one runs.
                    title.orchestrator = seat
                    body.append(title)
                }
                var board = entry(.board)
                board.workspace = workspace
                body.append(board)
                if !workspace.isImplicit {
                    var conductor = entry(.orchestrator)
                    conductor.workspace = workspace
                    conductor.orchestrator = seat
                    body.append(conductor)
                }
                for id in group.worktrees where byID[id] != nil { body.append(entry(.worktree(id))) }
            }
            unclaimedIDs = grouped.unclaimed
        }
        // Filled in once every orchestrator row is known: Billing's may run
        // in a checkout Main's rows drew above it.
        let depth = body.contains { if case .workspace = $0.kind { true } else { false } } ? 1 : 0
        for var row in body {
            if case .worktree(let id) = row.kind { row.worktree = byID[id]?.without(drawn) }
            row.depth = depth
            rows.append(row)
        }
        if !unclaimedIDs.isEmpty {
            var unclaimed = entry(.unclaimed(count: unclaimedIDs.count))
            unclaimed.worktrees = unclaimedIDs.compactMap { byID[$0]?.without(drawn) }
            unclaimed.depth = depth
            rows.append(unclaimed)
        }
        if !hidden.isEmpty {
            var group = entry(.hidden(count: hidden.count))
            group.worktrees = hidden.map { $0.without(drawn) }
            group.depth = depth
            rows.append(group)
        }
        return rows
    }

    /// Whether an orchestrator row is a hit for `query`: by what its pane is
    /// called, as a terminal in a worktree row is (`Worktree.matches`).
    private static func matches(_ terminal: Terminal, _ query: String) -> Bool {
        query.isEmpty || terminal.label.lowercased().contains(query.lowercased())
    }

    /// Who `workspace`'s orchestrator row shows: the runner's live seat,
    /// `WorkspaceSummary.orchestrator`, first. Only when the runner names
    /// none is a terminal taken by its role, and then only a live one — a
    /// workspace can hold a stopped orchestrator beside the one that
    /// replaced it, and the row must never show the stopped one over it.
    private static func orchestrator(
        of workspace: WorkspaceSummary, host: String, in fleet: Fleet
    ) -> BoardPane? {
        if let seat = workspace.orchestrator, let pane = pane(seat, host: host, in: fleet) {
            return pane
        }
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

    /// A terminal on `host`, with the worktree it is in.
    private static func pane(_ terminal: String, host: String, in fleet: Fleet) -> BoardPane? {
        for worktree in fleet.worktrees where (worktree.host ?? "") == host {
            if let found = worktree.terminals.first(where: { $0.id == terminal }) {
                return BoardPane(terminal: found, worktree: worktree)
            }
        }
        return nil
    }

    /// The orchestrator row `selection` stands for, if it is one: the pane
    /// an orchestrator row drew, in the checkout it runs in.
    ///
    /// Taken from the rows rather than from the terminal's role, so it is the
    /// same decision the sidebar made: a stopped orchestrator the runner no
    /// longer seats is drawn among its checkout's terminals, and is framed as
    /// one of them.
    static func orchestratorRow(for selection: Selection?, in rows: [SidebarEntry]) -> SidebarEntry? {
        guard case .terminal(let host, let worktree, let terminal) = selection else { return nil }
        return rows.first {
            $0.kind == .orchestrator && $0.host == host
                && $0.orchestrator?.terminal.id == terminal && $0.orchestrator?.worktree.id == worktree
        }
    }

    /// What the detail pane draws around a selected pane: which of its
    /// worktree's layouts the bar offers, and the window's title.
    ///
    /// A worktree's own pane gets that worktree's own layouts — `ownLayouts`,
    /// without the orchestrators' windows — and its title. A pane in a layout that isn't one of those (a shell moved into
    /// an orchestrator's window) gets that layout alone.
    ///
    /// An orchestrator's pane is in the checkout only because that is where
    /// the runner opens it: it gets its own layout and no bar — the
    /// checkout's other layouts are Main's shells and the other workspaces'
    /// orchestrators — and it is titled with its workspace, as the board is.
    static func detailFrame(
        _ worktree: Worktree, layouts: [PaneGroup]?, holding group: PaneGroup, seat: SidebarEntry?
    ) -> (groups: [PaneGroup], title: String, subtitle: String) {
        guard let seat, let workspace = seat.workspace else {
            let own = ownLayouts(layouts ?? [group], of: worktree)
            let groups = own.contains { $0.id == group.id } ? own : [group]
            return (groups, worktree.windowTitle, worktree.windowSubtitle)
        }
        let subtitle = [seat.project, "Orchestrator", seat.host.isEmpty ? nil : seat.host]
            .compactMap { $0 }
            .joined(separator: " · ")
        return ([group], workspace.name, subtitle)
    }

    /// The orchestrators the runner lists in `worktree`, by their role:
    /// what `ownLayouts` leaves out of the checkout they run in.
    ///
    /// By role rather than by the orchestrator rows the sidebar draws. Those
    /// decide which orchestrators get a row of their own, and there are none
    /// when the fleet read carries no workspaces for the runner — an older
    /// CLI, or before the first read — while the orchestrators' windows are
    /// still in the checkout's session. `worktree` is the runner's, not a
    /// row's: a row's has the orchestrators drawn elsewhere taken out.
    static func orchestrators(in worktree: Worktree) -> Set<String> {
        Set(worktree.terminals.filter(\.isOrchestrator).map(\.id))
    }

    /// `worktree`'s layouts without its orchestrators' windows.
    ///
    /// The runner opens every workspace's orchestrator as a tmux window in
    /// the main checkout's session, so the checkout's layouts include
    /// Billing's orchestrator. It is reached through its own row; offered in
    /// the checkout's bar, or picked as the checkout's layout because tmux
    /// calls it the active window since it was last focused, it would put
    /// Billing's orchestrator under Main's checkout.
    ///
    /// A stopped orchestrator the runner no longer seats is drawn among the
    /// checkout's terminals, and its window is left out all the same:
    /// selecting it shows that window alone. See `detailFrame`.
    static func ownLayouts(_ groups: [PaneGroup], of worktree: Worktree) -> [PaneGroup] {
        let orchestrators = orchestrators(in: worktree)
        guard !orchestrators.isEmpty else { return groups }
        return groups.filter { !$0.terminals.contains(where: orchestrators.contains) }
    }

    /// The layout selecting a worktree's own row shows: the active one of its
    /// own layouts, else the first of them. Nil when it has none.
    static func shownLayout(_ groups: [PaneGroup]?, of worktree: Worktree) -> PaneGroup? {
        let own = ownLayouts(groups ?? [], of: worktree)
        return own.first { $0.isActive } ?? own.first
    }

    /// The pane ⌃B and a digit focuses: that pane of the layout on screen,
    /// counted in tmux's pane order, or nil past its last.
    ///
    /// Picked here rather than by the runner's `layout focus --pane`, which
    /// counts in the window tmux calls active — an orchestrator's, if it was
    /// the last one focused, while the checkout's own layout is on screen.
    static func pane(numbered number: Int, in group: PaneGroup?) -> PaneRect? {
        guard let group, number >= 1, number <= group.panes.count else { return nil }
        return group.panes[number - 1]
    }

    /// The layout ⌃B n or ⌃B p lands on from `current`: the next or previous
    /// of `groups`, wrapping, or nil when there is nowhere else to go.
    ///
    /// Stepped here, through the layouts the bar offers, rather than by the
    /// runner's `layout select --next`, which walks every window in the
    /// session — from the main checkout, into the orchestrators'.
    static func layout(stepping step: Int, from current: String?, in groups: [PaneGroup]) -> PaneGroup? {
        guard groups.count > 1, let current, let index = groups.firstIndex(where: { $0.id == current })
        else { return nil }
        return groups[((index + step) % groups.count + groups.count) % groups.count]
    }

    /// The workspace `selection` is in, on its runner, as far as the fleet
    /// says: a board's own, the orchestrator's for its row, and otherwise
    /// the worktree's owner, or nil for an unclaimed worktree.
    private static func workspaceID(
        of selection: Selection, in fleet: Fleet
    ) -> (host: String, worktree: Worktree?, id: String?)? {
        switch selection {
        case .board(let host, let id):
            return (host, nil, id)
        case .worktree(let host, let id), .terminal(let host, let id, _):
            guard
                let worktree = fleet.worktrees.first(where: { ($0.host ?? "") == host && $0.id == id })
            else { return nil }
            if case .terminal(_, _, let terminal) = selection,
                let conductor = worktree.terminals.first(where: { $0.id == terminal }),
                conductor.isOrchestrator, let workspace = conductor.workspace
            {
                // Drawn under its own workspace, which need not be the
                // checkout's owner.
                return (host, worktree, workspace)
            }
            return (host, worktree, worktree.workspace)
        }
    }

    /// Whose board ⇧⌘B opens from `selection`: the workspace it is in, or
    /// Main's when that is an unclaimed worktree — the board every
    /// repository has. On a runner without workspaces, the repository's one
    /// board.
    ///
    /// Nil when the selection isn't in the fleet, and for a worktree on a
    /// runner without workspaces whose CLI sent no repository id — the
    /// caller then looks its repository up by name.
    static func boardWorkspace(for selection: Selection, in fleet: Fleet) -> WorkspaceSummary? {
        guard let found = workspaceID(of: selection, in: fleet) else { return nil }
        let (host, worktree, id) = found
        guard let listed = fleet.runnerWorkspaces[host] else {
            // No workspaces: a board selection's id is its repository's.
            let repository = worktree.map(\.repositoryID) ?? id
            return repository.map(WorkspaceSummary.implicit(repository:))
        }
        if let id, let found = listed.first(where: { $0.id == id }) { return found }
        guard let worktree else { return nil }
        return listed.first { $0.isMain && $0.repository != nil && $0.repository == worktree.repositoryID }
    }

    /// The workspace a new worktree in `repository` on `host` is claimed for:
    /// the one `selection` is in, when it is in that repository on that
    /// runner, and otherwise the repository's Main — an unclaimed worktree,
    /// another repository or runner, or nothing selected. Never left
    /// unclaimed where the runner has workspaces: a pane opened in an
    /// unclaimed worktree has no workspace, so nothing done there could ever
    /// claim it. Nil only on a runner without workspaces, which has nothing
    /// to claim for.
    static func claim(
        newWorktreeIn repository: String, on host: String, from selection: Selection?,
        in fleet: Fleet
    ) -> String? {
        guard let listed = fleet.runnerWorkspaces[host] else { return nil }
        if let selection, let found = workspaceID(of: selection, in: fleet), found.host == host,
            let id = found.id,
            let workspace = listed.first(where: { $0.id == id }), workspace.repository == repository
        {
            return workspace.id
        }
        return listed.first { $0.isMain && $0.repository == repository }?.id
    }
}
