import AgentKit
import Foundation

// The sidebar's shape (spec §4.5): Needs You at the top, then each
// repository's workspaces, one row each, with a workspace's worktrees one
// click down under its Worktrees disclosure, and the worktrees no workspace
// owns in one collapsed Unclaimed group per repository.
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
        /// A workspace, by its name: a place you select, with its
        /// orchestrator's status and its needs-you count. On a runner
        /// without workspaces, the repository's implicit one.
        case workspace(String)
        /// A workspace's Worktrees disclosure, with how many it holds.
        case worktrees(count: Int)
        /// A worktree inside an open Worktrees disclosure, by its id.
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
    /// The workspace a workspace row, its disclosure or a worktree inside it
    /// is for. An implicit one on a runner without workspaces.
    var workspace: WorkspaceSummary?
    /// The worktree a worktree row draws, without its orchestrators.
    var worktree: Worktree?
    /// The workspace's orchestrator, and the worktree it runs in: its seat.
    /// Nil with none running.
    var orchestrator: BoardPane?
    /// A repository header's shown worktrees, or a Worktrees disclosure's,
    /// or an Unclaimed or Hidden group's.
    var worktrees: [Worktree] = []
    /// How many steps in from the repository's header this row is drawn.
    var depth = 0

    /// Which repository this row is under, on which runner: by uuid where the
    /// CLI sends one, so two repositories that share a name are two groups.
    var group: String { "\(host)\u{1}\(repositoryID ?? project)" }

    /// What a repository's collapsed state and its Hidden group are
    /// remembered by. The display name, as before workspaces, so nobody's
    /// sidebar reopens differently after an update.
    var collapseKey: String { "\(host)\u{1}\(project)" }

    /// What a workspace's Worktrees disclosure is remembered open by, in
    /// `sidebar.openWorktrees`.
    static func openKey(host: String, workspace: String) -> String { "\(host)\u{1}\(workspace)" }

    /// A workspace row's menu, from ov-60's: Show Board, Start or Replace
    /// Orchestrator, Show Charter. None for a repository's implicit
    /// workspace, which has no orchestrator or charter.
    var menu: [WorkspaceMenu.Item] {
        guard case .workspace = kind, let workspace, !workspace.isImplicit else { return [] }
        return WorkspaceMenu.items(hasBoard: true, hasOrchestrator: orchestrator != nil)
    }

    var id: String {
        switch kind {
        case .repository: return group
        case .workspace: return "\(group)\u{1}workspace\u{1}\(workspace?.id ?? "")"
        case .worktrees: return "\(group)\u{1}worktrees\u{1}\(workspace?.id ?? "")"
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
    ///   project of the same name, and they are not the same project.
    /// - A repository lists its workspaces, never its worktrees: those are
    ///   under each workspace's Worktrees, drawn only while it's `open`
    ///   (keyed by `SidebarEntry.openKey`), and under Unclaimed and Hidden.
    /// - A runner without workspaces (`fleet.runnerWorkspaces` has no key for
    ///   it) has one implicit workspace per repository.
    /// - With a `query`, a workspace is listed when its name, its
    ///   orchestrator or one of its worktrees matches, and its Worktrees
    ///   open on the matches. A repository with no match is left out: a
    ///   header with nothing under it would look like a hit.
    /// - `silentHosts` are runners that have contributed nothing yet; each
    ///   gets a header, as before.
    static func sidebarRows(
        fleet: Fleet, query: String = "", silentHosts: [String] = [], open: (String) -> Bool = { _ in false }
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
            let entries = repositoryRows(
                group.host, group.project, group.repositoryID, group.worktrees, fleet, query, open)
            // Only a header: nothing in this repository matched the search.
            if entries.count > 1 { rows += entries }
        }
        if query.isEmpty {
            for host in silentHosts {
                rows.append(SidebarEntry(kind: .repository, host: host, project: "", repositoryID: nil))
            }
        }
        return rows
    }

    /// One repository's rows: its header, each workspace's row and, open or
    /// not, its Worktrees, then Unclaimed and Hidden.
    private static func repositoryRows(
        _ host: String, _ project: String, _ repositoryID: String?, _ all: [Worktree],
        _ fleet: Fleet, _ query: String, _ open: (String) -> Bool
    ) -> [SidebarEntry] {
        // The runner's order, and nothing else: with rows draggable, a rule
        // here would silently outrank the one the person dragging expressed.
        func entry(_ kind: SidebarEntry.Kind) -> SidebarEntry {
            SidebarEntry(kind: kind, host: host, project: project, repositoryID: repositoryID)
        }
        let listed = fleet.runnerWorkspaces[host]
        let workspaces: [WorkspaceSummary] = {
            guard project != "Ungrouped" else { return [] }
            guard let listed else { return repositoryID.map { [.implicit(repository: $0)] } ?? [] }
            return listed.filter { $0.repository != nil && $0.repository == repositoryID }
        }()

        // The orchestrators seated in their workspaces' rows, by terminal
        // id: these, and only these, are left out of the worktree rows, so
        // each is drawn exactly once — and an orchestrator-role terminal no
        // workspace seats (a stopped one) stays where it is.
        var seats: [String: BoardPane] = [:]
        for workspace in workspaces where !workspace.isImplicit {
            seats[workspace.id] = WorkspaceScreen.orchestrator(of: workspace, host: host, in: fleet)
        }
        let drawn = Set(seats.values.map(\.terminal.id))
        let searching = !query.isEmpty
        let matched = all.filter { $0.without(drawn).matches(query) }
        let shown = matched.filter { !$0.isHidden }
        let hidden = matched.filter(\.isHidden)

        var header = entry(.repository)
        header.worktrees = all.filter { !$0.isHidden }
        var rows = [header]
        let byID = Dictionary(shown.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })

        if workspaces.isEmpty {
            // No repository, so no workspaces: its worktrees, loose.
            if !shown.isEmpty {
                var loose = entry(.unclaimed(count: shown.count))
                loose.worktrees = shown.map { $0.without(drawn) }
                loose.depth = 1
                rows.append(loose)
            }
        } else {
            // Every worktree the workspace owns, matched or not, so a
            // workspace whose name matches still lists its worktrees.
            let everyone = all.filter { !$0.isHidden }
            // A repository's implicit workspace, on a runner without
            // workspaces, owns every worktree in it.
            let grouped = WorkspaceGrouping.group(
                repository: repositoryID ?? "", workspaces: workspaces,
                worktrees: everyone.map { (id: $0.id, workspace: listed == nil ? repositoryID : $0.workspace) },
                orchestrators: [:])
            let owned = Dictionary(everyone.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })

            for group in grouped.workspaces {
                let workspace = group.workspace
                let seat = seats[workspace.id]
                let nameMatches = searching && workspace.name.lowercased().contains(query.lowercased())
                let seatMatches = searching && (seat.map { Self.matches($0.terminal, query) } ?? false)
                let hits = group.worktrees.filter { byID[$0] != nil }
                if searching && hits.isEmpty && !nameMatches && !seatMatches { continue }
                var row = entry(.workspace(workspace.isImplicit ? "Main" : workspace.name))
                row.workspace = workspace
                row.orchestrator = seat
                row.depth = 1
                rows.append(row)

                // Its worktrees: all of them, or while searching, the hits
                // (all of them for a workspace found by its name).
                let listedIDs = searching && !nameMatches ? hits : group.worktrees
                let worktrees = listedIDs.compactMap { owned[$0]?.without(drawn) }
                guard !worktrees.isEmpty else { continue }
                var disclosure = entry(.worktrees(count: worktrees.count))
                disclosure.workspace = workspace
                disclosure.worktrees = worktrees
                disclosure.depth = 2
                rows.append(disclosure)
                let isOpen = open(SidebarEntry.openKey(host: host, workspace: workspace.id)) || (searching && !hits.isEmpty)
                guard isOpen else { continue }
                for worktree in worktrees {
                    var line = entry(.worktree(worktree.id))
                    line.workspace = workspace
                    line.worktree = worktree
                    line.depth = 2
                    rows.append(line)
                }
            }
            let unclaimed = grouped.unclaimed.compactMap { byID[$0]?.without(drawn) }
            if !unclaimed.isEmpty {
                var group = entry(.unclaimed(count: unclaimed.count))
                group.worktrees = unclaimed
                group.depth = 1
                rows.append(group)
            }
        }
        if !hidden.isEmpty {
            var group = entry(.hidden(count: hidden.count))
            group.worktrees = hidden.map { $0.without(drawn) }
            group.depth = 1
            rows.append(group)
        }
        return rows
    }

    /// What Move to Workspace ▸ offers for `worktree`: exactly the
    /// workspaces a drag of its row onto their rows would move it to
    /// (`dropMeaning`), so the menu and the drag can't disagree.
    static func moveTargets(for worktree: Worktree, in fleet: Fleet, assigns: Bool) -> [WorkspaceSummary] {
        (fleet.runnerWorkspaces[worktree.host ?? ""] ?? []).filter {
            dropMeaning(worktree, onto: .workspace($0.id), in: fleet, assigns: assigns) == .assign($0)
        }
    }

    /// Whether an orchestrator is a hit for `query`: by what its pane is
    /// called, as a terminal in a worktree row is (`Worktree.matches`).
    private static func matches(_ terminal: Terminal, _ query: String) -> Bool {
        query.isEmpty || terminal.label.lowercased().contains(query.lowercased())
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
    /// selecting it shows that window alone. See `WorkspaceScreen.shown`.
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
        case .needsYou:
            return nil
        case .workspace(let host, let id, _):
            return (host, nil, id)
        case .looseWorktree(let host, let id, _):
            guard
                let worktree = fleet.worktrees.first(where: { ($0.host ?? "") == host && $0.id == id })
            else { return nil }
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
