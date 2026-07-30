import SwiftUI

struct ContentView: View {
    @StateObject private var client = DaemonClient()
    @StateObject private var service = ServiceRegistration()
    @State private var selection: Selection?
    @State private var expanded: Set<String> = []
    @State private var pollTask: Task<Void, Never>?

    @State private var showNewWorkspace = false
    @State private var showAddRepository = false
    @State private var showShortcuts = false
    @State private var query = ""
    @State private var showQuickCreate = false
    @AppStorage("tasks.lastProject") private var lastProject = ""
    @FocusState private var searchFocused: Bool
    @State private var removeWorkspace: Workspace?
    @State private var showResumeBranch = false
    /// Bound so ⌘B can move it. `.automatic` lets the system decide the first
    /// time, which is the right answer for a window that has never been sized.
    @State private var columns = NavigationSplitViewVisibility.automatic

    /// What the detail pane is showing.
    enum Selection: Hashable {
        case workspace(String)
        case terminal(workspace: String, terminal: String)
    }

    var body: some View {
        NavigationSplitView(columnVisibility: $columns) {
            sidebar
        } detail: {
            detail
        }
        .task {
            Notifier.shared.requestAuthorisation()
            await client.refresh()
            await client.refreshRepositories()
            await client.refreshRoots()
            await client.refreshLayouts()
            selectFirstRunningTerminal()
            // Pushed, not polled. The daemon derives once for every client and
            // sends only what changed, so a quiet fleet costs nothing and a
            // question from an agent arrives at once instead of up to a poll
            // interval later.
            client.startEvents()
        }
        .onDisappear { client.stopEvents() }
        .onCommand { command in run(command) }
        .onTileCommand { command in Task { await tile(command) } }
        .onSelectIndex { index in selectTerminal(at: index) }
        .onChange(of: client.layouts) { _, _ in followLayoutFocus() }
        .onChange(of: client.fleet) { _, _ in
            // One rule for every way a terminal can disappear: exiting on its
            // own, being closed here, being closed from a phone, or its
            // workspace being archived. Hooking each path separately meant the
            // common one — you press Ctrl-D in the terminal you are looking at —
            // left the selection pointing at something that no longer existed.
            healSelection()
        }
        .onChange(of: selection) { _, new in
            // Opening a terminal is what ends `done`. Being listed is not being
            // read, so this is deliberately tied to selection.
            if case .terminal(_, let id) = new,
                let terminal = allTerminals.first(where: { $0.id == id })
            {
                Task { await client.markSeen(terminal.short) }
            }
        }
        .sheet(isPresented: $showShortcuts) { ShortcutsSheet() }
        .sheet(isPresented: $showResumeBranch) {
            ResumeBranch(
                projects: client.repositories,
                project: $lastProject,
                load: { await client.branches(project: $0) },
                onAdopt: { branch, project, preset in
                    lastProject = project
                    resume(branch: branch.name, project: project, agent: preset)
                }
            )
        }
        // An overlay, not a sheet. A sheet dims the window and takes it over,
        // which is the wrong weight for something you use several times in a
        // row and want to see the results of behind.
        .overlay(alignment: .top) {
            if showQuickCreate {
                QuickCreate(
                    projects: client.repositories,
                    project: $lastProject,
                    onSubmit: { description, project, preset in
                        lastProject = project
                        startTask(description: description, project: project, agent: preset)
                    },
                    onResume: {
                        showQuickCreate = false
                        showResumeBranch = true
                    },
                    onClose: { showQuickCreate = false }
                )
                .padding(.top, 14)
                .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.snappy(duration: 0.16), value: showQuickCreate)
        .onReceive(
            NotificationCenter.default.publisher(
                for: NSApplication.didBecomeActiveNotification)
        ) { _ in
            // The user may have approved the login item in System Settings
            // while we were in the background; nothing tells us but this.
            service.refresh()
        }
        .sheet(isPresented: $showAddRepository) {
            AddRepositorySheet(
                roots: client.roots,
                onAddRoot: { path in
                    let failure = await client.addRoot(path)
                    await client.refreshRoots()
                    return failure
                },
                onRegister: { path in await client.registerRepository(path) }
            )
        }
        .sheet(isPresented: $showNewWorkspace) {
            NewWorkspaceSheet(repositories: client.repositories) { repo, task, branch, base in
                await client.createWorkspace(repo: repo, task: task, branch: branch, base: base)
            }
        }
        .sheet(item: $removeWorkspace) { ws in
            RemoveWorkspaceSheet(
                workspace: ws,
                hasRunningTerminals: ws.terminals.contains {
                    StateKind.parse($0.state) == .running
                }
            ) { typed in
                await client.removeWorktree(ws.short, confirm: typed)
                if case .terminal(let w, _) = selection, w == ws.id { selection = nil }
            }
        }
    }

    // MARK: - Sidebar

    /// Worktrees matching the search, grouped by project.
    ///
    /// Grouped rather than filtered by host: you work across machines at once,
    /// and a host picker would make a remote agent something to go and look for
    /// instead of something already in the list. Where a project name appears
    /// on more than one machine, the host is appended to tell them apart —
    /// which is the only time it needs saying.
    private var groups: [(String, [Workspace])] {
        let visible = client.fleet.workspaces.filter { $0.matches(query) }
        let hosts = Set(visible.map { $0.host ?? "" })
        var order: [String] = []
        var byProject: [String: [Workspace]] = [:]

        for workspace in visible {
            let project = (workspace.repository ?? "").isEmpty
                ? "Ungrouped" : workspace.repository!
            let host = workspace.host ?? ""
            let key = hosts.count > 1 && !host.isEmpty ? "\(project) · \(host)" : project
            if byProject[key] == nil { order.append(key) }
            byProject[key, default: []].append(workspace)
        }
        return order.map { ($0, byProject[$0] ?? []) }
    }

    private var sidebar: some View {
        VStack(spacing: 0) {
            sidebarHeader
            searchField

            if client.fleet.workspaces.isEmpty {
                fleetPlaceholder
                Spacer(minLength: 0)
            } else if groups.isEmpty {
                VStack(spacing: 6) {
                    Text("Nothing matches").font(.callout.weight(.medium))
                    Text("\u{201c}\(query)\u{201d}")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 32)
                Spacer(minLength: 0)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(groups, id: \.0) { project, workspaces in
                            ProjectHeader(name: project, count: workspaces.count)
                            ForEach(workspaces) { ws in
                                WorkspaceSection(
                                    workspace: ws,
                                    isExpanded: expanded.contains(ws.id),
                                    selection: $selection,
                                    onToggle: { toggle(ws.id) },
                                    onNewTerminal: { newTerminal(in: ws) },
                                    onArchive: { Task { await client.archiveWorkspace(ws.short) } },
                                    onRemove: { removeWorkspace = ws },
                                    onTerminalAction: { term, action in
                                        Task { await run(action, on: term) }
                                    },
                                    tiled: Set(client.activeGroup(ws.id)?.terminals ?? [])
                                )
                            }
                        }
                    }
                    .padding(.horizontal, 8)
                    .padding(.bottom, 10)
                }
            }

            statusBar
        }
        .navigationSplitViewColumnWidth(min: 268, ideal: 320, max: 440)
    }

    /// Search, because worktrees are unbounded.
    ///
    /// It matches terminals too, so typing an agent's name finds the worktree
    /// containing it — which is how you reach an agent on another machine
    /// without going looking for the machine.
    private var searchField: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
            TextField("Search worktrees and agents", text: $query)
                .textFieldStyle(.plain)
                .font(.system(size: 12))
                .focused($searchFocused)
            if !query.isEmpty {
                Button { query = "" } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(0.05)))
        .padding(.horizontal, 14)
        .padding(.bottom, 6)
    }

    /// Everything waiting on you, across every project and every machine.
    ///
    /// The one number the app exists to produce, and it deliberately spans
    /// hosts: an agent blocked on a machine in another room is exactly as
    /// urgent as one on this desk.
    private var attentionCount: Int {
        client.fleet.workspaces.flatMap(\.terminals).filter(\.status.wantsAttention).count
    }

    private var sidebarHeader: some View {
        HStack(spacing: 8) {
            Text("Fleet").font(.headline)
            if client.busy { ProgressView().controlSize(.mini) }

            if attentionCount > 0 {
                // A dot and a number, not a filled capsule. A solid block of
                // colour in the header shouts louder than the row it points at,
                // which leaves it pointing at itself.
                Button {
                    AppCommand.nextAttention.post()
                } label: {
                    AttentionBadge(count: attentionCount)
                }
                .buttonStyle(.plain)
                .help("\(attentionCount) waiting on you — click to jump there")
            }

            Spacer()
            // A menu rather than a button, because "add a repository" has to be
            // reachable at all times. It used to live only in the empty state,
            // so once you had one workspace there was no way to add a second
            // repository without dropping to the terminal.
            Menu {
                Button("New workspace…") { showNewWorkspace = true }
                    .disabled(client.repositories.isEmpty)
                Divider()
                Button("Add repository…") { showAddRepository = true }
            } label: {
                Image(systemName: "plus").font(.system(size: 12, weight: .semibold))
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("Add a workspace or a repository")
        }
        // Aligned to the same rail as the workspace names below it, so the
        // section title heads its column instead of sitting off to one side of
        // it. The vstack's own padding is added on top, hence the subtraction.
        .padding(.leading, Grid.rail + 8)
        .padding(.trailing, 14)
        .padding(.top, 12)
        .padding(.bottom, 8)
    }

    /// Shown only once a fleet has actually been read.
    ///
    /// Telling someone they have no workspaces when the truth is that we could
    /// not read them is worse than saying nothing, because it sends them to
    /// create one they already have.
    @ViewBuilder
    private var fleetPlaceholder: some View {
        if client.hasLoaded {
            emptyFleet
        } else if let error = client.lastError {
            VStack(spacing: 10) {
                Image(systemName: "exclamationmark.triangle")
                    .font(.system(size: 26))
                    .foregroundStyle(.orange)
                Text("Could not read the fleet").font(.callout.weight(.medium))
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .textSelection(.enabled)
                Button("Try again") {
                    Task { await client.refresh() }
                }
                .padding(.top, 4)
            }
            .padding(.horizontal, 26)
            .padding(.vertical, 34)
        } else {
            ProgressView()
                .controlSize(.small)
                .padding(.vertical, 40)
        }
    }

    private var emptyFleet: some View {
        VStack(spacing: 10) {
            Image(systemName: "rectangle.stack")
                .font(.system(size: 26))
                .foregroundStyle(.tertiary)
            Text("No workspaces").font(.callout.weight(.medium))
            Text("A workspace is one worktree and branch for one task.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

            if client.repositories.isEmpty {
                Text("Add a repository to get started.")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .multilineTextAlignment(.center)
                Button("Add repository…") { showAddRepository = true }.padding(.top, 4)
            } else {
                Button("New workspace") { showNewWorkspace = true }.padding(.top, 4)
            }
        }
        .padding(.horizontal, 26)
        .padding(.vertical, 34)
    }

    /// Whether the daemon starts at login.
    ///
    /// Lives in Settings now. It is a preference you set once, not a status you
    /// watch, and a permanent control in the status bar made a piece of
    /// configuration look like live information.
    @ViewBuilder
    private var loginItemToggle: some View {
        switch service.state {
        case .registered:
            Button { service.unregister() } label: {
                Label("Starts at login", systemImage: "bolt.fill").font(.system(size: 10))
            }
            .buttonStyle(.borderless)
            .foregroundStyle(.secondary)
            .help("The daemon starts at login, so this host stays reachable. Click to stop.")

        case .notRegistered:
            Button { service.register() } label: {
                Label("Start at login", systemImage: "bolt").font(.system(size: 10))
            }
            .buttonStyle(.borderless)
            .foregroundStyle(.secondary)
            .help("Keep this host reachable without opening Overnight first.")

        case .awaitingApproval:
            Button { service.register() } label: {
                Label("Approve in Settings", systemImage: "exclamationmark.triangle")
                    .font(.system(size: 10))
            }
            .buttonStyle(.borderless)
            .foregroundStyle(.orange)
            .help("macOS needs you to enable Overnight under Login Items.")

        case .unavailable(let why):
            Image(systemName: "bolt.slash")
                .font(.system(size: 10))
                .foregroundStyle(.tertiary)
                .help(why)
        }
    }

    private var statusBar: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 7) {
                Circle()
                    .fill(client.fleet.runtimeHealthy ? Color.green : Color.orange)
                    .frame(width: 7, height: 7)
                Text(
                    client.fleet.runtimeHealthy
                        ? "\(client.fleet.livePanes) live"
                        : "tmux unavailable"
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                Spacer()

                Button {
                    Task {
                        await client.refresh()
                        await client.refreshRepositories()
                    }
                } label: {
                    Image(systemName: "arrow.clockwise").font(.system(size: 11))
                }
                .buttonStyle(.borderless)
                .help("Refresh")
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 9)
        }
    }

    // MARK: - Detail

    @ViewBuilder
    private var detail: some View {
        switch selection {
        case .terminal(let wsID, let termID):
            // Tiled or solo, decided by whether the selected terminal is in the
            // group on screen. Selecting a backgrounded terminal shows it alone,
            // which is what makes the fourth agent reachable without disturbing
            // the three you have arranged.
            if let ws = workspace(wsID), let group = client.activeGroup(wsID),
                group.terminals.contains(termID)
            {
                TileView(
                    group: group,
                    workspace: ws,
                    binary: client.cliPath,
                    environment: client.cliEnvironment,
                    onFocus: { id in
                        selection = .terminal(workspace: wsID, terminal: id)
                        Task { await client.layout(ws, ["focus", id]) }
                    },
                    onResize: { short, cols, rows in
                        await client.resize(terminal: short, columns: cols, rows: rows)
                    }
                )
            } else if let ws = workspace(wsID),
                let term = ws.terminals.first(where: { $0.id == termID })
            {
                TerminalPane(
                    terminal: term,
                    workspace: ws,
                    binary: client.cliPath,
                    environment: client.cliEnvironment,
                    onGeometry: { cols, rows in
                        await client.resize(terminal: term.short, columns: cols, rows: rows)
                    },
                    onAction: { action in Task { await run(action, on: term) } }
                )
            } else {
                placeholder
            }

        case .workspace(let wsID):
            // A worktree with a layout shows the layout. The card list was the
            // right answer when a workspace had no arrangement of its own; once
            // it does, showing a summary of the panes instead of the panes is a
            // click in the way.
            if let ws = workspace(wsID), let group = client.activeGroup(wsID),
                !group.terminals.isEmpty
            {
                TileView(
                    group: group,
                    workspace: ws,
                    binary: client.cliPath,
                    environment: client.cliEnvironment,
                    onFocus: { id in
                        selection = .terminal(workspace: wsID, terminal: id)
                        Task { await client.layout(ws, ["focus", id]) }
                    },
                    onResize: { short, cols, rows in
                        await client.resize(terminal: short, columns: cols, rows: rows)
                    }
                )
            } else if let ws = workspace(wsID) {
                WorkspaceDetail(
                    workspace: ws,
                    onTile: { Task { await tile(.tile) } },
                    onNewTerminal: { newTerminal(in: ws) },
                    onArchive: { Task { await client.archiveWorkspace(ws.short) } },
                    onRemove: { removeWorkspace = ws },
                    onOpenTerminal: { t in
                        selection = .terminal(workspace: ws.id, terminal: t.id)
                    }
                )
            } else {
                placeholder
            }

        case nil:
            placeholder
        }
    }

    private var placeholder: some View {
        ContentUnavailableView {
            Label("Select a workspace", systemImage: "rectangle.split.3x1")
        } description: {
            Text("Each workspace is one worktree and branch for one task.")
        } actions: {
            if !client.repositories.isEmpty {
                Button("New workspace") { showNewWorkspace = true }
            }
        }
    }

    // MARK: - Behaviour

    private func run(_ action: TerminalAction, on term: Terminal) async {
        switch action {
        case .restart: await client.restart(terminal: term.short)
        case .dismissLost: await client.dismissLost(terminal: term.short)
        case .stop: await client.stop(terminal: term.short)
        }
    }

    private func workspace(_ id: String) -> Workspace? {
        client.fleet.workspaces.first { $0.id == id }
    }

    private func toggle(_ id: String) {
        if expanded.contains(id) { expanded.remove(id) } else { expanded.insert(id) }
    }

    /// Kept for the one case that still wants it: opening a worktree you just
    /// selected. Launching no longer expands everything — with hundreds of
    /// worktrees that produced a wall of terminals and hid the summary line
    /// that collapsing exists to show.
    private func expandAll() {
        expanded.formUnion(client.fleet.workspaces.map(\.id))
    }

    /// Land on something usable rather than an empty pane.
    /// Where to land on launch.
    ///
    /// Whatever wants you first, wherever it is — including on another machine.
    /// Falling back to "the first running terminal" would open a fleet on
    /// something arbitrary while an agent two projects down waits for an answer.
    private func selectFirstRunningTerminal() {
        guard selection == nil else { return }

        for ws in client.fleet.workspaces {
            if let t = ws.terminals.first(where: { $0.status.wantsAttention }) {
                expanded.insert(ws.id)
                selection = .terminal(workspace: ws.id, terminal: t.id)
                return
            }
        }
        for ws in client.fleet.workspaces {
            if let t = ws.terminals.first(where: { StateKind.parse($0.state) == .running }) {
                expanded.insert(ws.id)
                selection = .terminal(workspace: ws.id, terminal: t.id)
                return
            }
        }
        if let ws = client.fleet.workspaces.first {
            selection = .workspace(ws.id)
        }
    }

    // MARK: - Commands

    /// Every terminal in the fleet, in the order the sidebar shows them.
    ///
    /// One definition, so ⌘1 and a click select the same thing and ⌘] walks the
    /// list a user can actually see.
    private var allTerminals: [Terminal] {
        client.fleet.workspaces.flatMap(\.terminals)
    }

    private var selectedTerminal: (workspace: Workspace, terminal: Terminal)? {
        guard case .terminal(let workspaceID, let terminalID) = selection,
            let workspace = client.fleet.workspaces.first(where: { $0.id == workspaceID }),
            let terminal = workspace.terminals.first(where: { $0.id == terminalID })
        else { return nil }
        return (workspace, terminal)
    }

    // MARK: - Tiling

    /// The workspace a tiling keystroke acts on.
    private var tileTarget: Workspace? { currentWorkspace }

    /// Carry out a `⌃B`-prefixed command.
    ///
    /// Every one of them is a `layout` call and the reply is the workspace's
    /// whole layout, so there is nothing to reconcile here: the daemon decides
    /// what a zoom or a focus means, and it decides the same way for the CLI and
    /// for an agent. The only thing this file adds is geometry, for the two
    /// commands that need it.
    private func tile(_ command: TileCommand) async {
        guard let workspace = tileTarget else { return }
        let group = client.activeGroup(workspace.id)

        switch command {
        case .tile:
            let groups = await client.layout(workspace, ["tile"])
            // Land on the layout you just made rather than leaving the selection
            // pointing at a terminal that is now one pane of it.
            if let focused = groups.first(where: { $0.isActive })?.focused {
                selection = .terminal(workspace: workspace.id, terminal: focused)
            }

        case .untile:
            // Every pane out, which deletes the group. The terminals keep
            // running: un-tiling four agents must never be a way to stop four
            // agents.
            guard let group, !group.terminals.isEmpty else { return }
            await client.layout(workspace, ["drop"] + group.terminals)

        case .zoom:
            await client.layout(workspace, ["zoom"])

        case .focusNext:
            await client.layout(workspace, ["focus", "--next"])
        case .focusPrevious:
            await client.layout(workspace, ["focus", "--prev"])

        case .focus(let direction):
            // The one place the client's geometry is authoritative, because it
            // is the only place that knows where the panes actually are. The
            // daemon is then told a plain terminal id, so there is no second
            // copy of the layout maths on the host to disagree with this one.
            guard let group, let from = group.focused,
                let index = group.terminals.firstIndex(of: from)
            else { return }
            let frames = TileGeometry.frames(
                count: group.terminals.count,
                preset: group.layout,
                ratio: group.share,
                in: CGRect(x: 0, y: 0, width: 1000, height: 700))
            guard
                let next = TileGeometry.neighbour(
                    of: index, direction: direction, frames: frames)
            else { return }
            await client.layout(workspace, ["focus", group.terminals[next]])

        case .focusIndex(let n):
            await client.layout(workspace, ["focus", "--pane", "\(n)"])

        case .cycle:
            await client.layout(workspace, ["cycle"])

        case .preset(let preset):
            await client.layout(workspace, ["preset", preset.rawValue])

        case .splitRight, .splitDown:
            // A split is a new terminal in the layout you are looking at. The
            // direction is the arrangement's business, so `%` and `"` set the
            // preset and then add a pane, which is what tmux's own splits amount
            // to once a layout is applied.
            let preset: TilePreset = command == .splitRight ? .mainVertical : .mainHorizontal
            if group == nil {
                // Nothing tiled yet: tile what is here first, or the new pane
                // would be alone in a group and look like nothing happened.
                await client.layout(workspace, ["tile"])
            }
            await client.layout(workspace, ["preset", preset.rawValue])
            await client.createTerminal(
                workspace: workspace.short,
                preset: "shell",
                title: "Terminal \(workspace.terminals.count + 1)",
                tile: true)
            await client.refreshLayout(workspace)

        case .breakPane:
            guard let focused = group?.focused else { return }
            await client.layout(workspace, ["drop", focused])
            selection = .terminal(workspace: workspace.id, terminal: focused)

        case .shiftForward:
            await client.layout(workspace, ["shift"])
        case .shiftBack:
            await client.layout(workspace, ["shift", "--back"])

        case .closePane:
            // tmux's `x`, and it means the same thing: the pane's process ends.
            // Routed through the existing close so there is one implementation of
            // what closing a terminal does.
            run(.closeTerminal)

        case .newGroup:
            await client.layout(workspace, ["group", "new"])
        case .nextGroup:
            await client.layout(workspace, ["group", "select", "--next"])
        case .previousGroup:
            await client.layout(workspace, ["group", "select", "--prev"])
        case .closeGroup:
            await client.layout(workspace, ["group", "close"])

        case .help:
            showShortcuts = true
        }
    }

    /// Follow the layout's focus when something else moved it.
    ///
    /// The CLI and an agent can both focus a pane, and when they do the app has
    /// to be looking at it — otherwise `overnight layout focus` from a script
    /// draws a border around a pane whose keystrokes still go somewhere else.
    private func followLayoutFocus() {
        guard case .terminal(let wsID, let termID) = selection,
            let group = client.activeGroup(wsID),
            let focused = group.focused,
            focused != termID,
            group.terminals.contains(termID)
        else { return }
        selection = .terminal(workspace: wsID, terminal: focused)
    }

    private func run(_ command: AppCommand) {
        switch command {
        case .newTerminal:
            // Creates immediately. There is no agent to choose: a terminal is a
            // shell, and whatever you run in it — `claude`, `codex`, a build —
            // is detected from the process, not declared in advance. Asking
            // first was a dialog whose answer was already knowable.
            if let workspace = currentWorkspace { newTerminal(in: workspace) }

        case .closeTerminal:
            guard let (_, terminal) = selectedTerminal else { return }
            Task {
                // Stop, then remove the record. Closing a terminal should leave
                // nothing behind — that is what closing means everywhere else.
                await client.stop(terminal: terminal.short)
                await client.removeTerminal(terminal.short)
                selectNeighbour(of: terminal)
            }

        case .nextTerminal: step(by: 1)
        case .previousTerminal: step(by: -1)

        case .nextAttention:
            // Straight to whatever is waiting on you. On a fleet of twenty this
            // is the difference between the app being useful and being a list.
            let ordered = allTerminals
            let start = ordered.firstIndex { $0.id == selectedTerminal?.terminal.id } ?? -1
            let rotated = ordered[(start + 1)...] + ordered[...max(start, 0)]
            if let next = rotated.first(where: { $0.agent.wantsAttention }) {
                select(next)
            }

        case .newWorkspace:
            // Only reachable with a project registered; the panel has nothing
            // to create into otherwise.
            if client.repositories.isEmpty {
                showAddRepository = true
            } else {
                if lastProject.isEmpty { lastProject = client.repositories[0].id }
                showQuickCreate = true
            }
        case .addRepository: showAddRepository = true
        case .reload: Task { await client.refresh() }
        case .showShortcuts: showShortcuts = true
        case .search: searchFocused = true

        case .toggleSidebar:
            // Two states, not three. `.automatic` is a starting position, and
            // cycling a user through it on the way to hidden would make the same
            // keystroke do different things on consecutive presses.
            columns = columns == .detailOnly ? .all : .detailOnly
        }
    }

    /// Start a task and go to it as soon as it exists.
    ///
    /// Deliberately does not wait for the agent to boot before selecting. The
    /// point is to be looking at the thing you asked for while it starts, not
    /// to stare at the old screen for ten seconds first.
    private func startTask(description: String, project: String, agent: String) {
        Task {
            let created = await client.startTask(
                project: project,
                description: description,
                agent: agent.isEmpty ? Preferences.shared.defaultAgent : agent)
            reveal(created)
        }
    }

    /// Pick up an existing branch and open an agent in it.
    ///
    /// Same landing as starting a task, because it is the same act from the
    /// user's side: there is now a worktree with an agent in it and you want to
    /// be looking at it. The only difference is where the code came from.
    private func resume(branch: String, project: String, agent: String) {
        Task {
            let created = await client.adoptBranch(
                project: project, branch: branch, agent: agent)
            reveal(created)
        }
    }

    /// Select a freshly created workspace, preferring its terminal.
    private func reveal(_ workspace: String?) {
        guard let workspace else { return }
        expanded.insert(workspace)
        if let found = client.fleet.workspaces.first(where: { $0.id == workspace }),
            let terminal = found.terminals.first
        {
            selection = .terminal(workspace: workspace, terminal: terminal.id)
        } else {
            selection = .workspace(workspace)
        }
    }

    /// Create a terminal and go straight to it.
    ///
    /// Selecting it afterwards matters: you made a terminal because you want to
    /// type in it, and leaving the selection where it was means a second click
    /// to get to the thing you just asked for.
    private func newTerminal(in workspace: Workspace) {
        // The ids that existed before, so the new one can be identified by
        // difference. Comparing whole `Terminal` values did not work: any of
        // them changing activity between the two reads also looked "new", so
        // the selection sometimes landed on a terminal the user did not create.
        let before = Set(workspace.terminals.map(\.id))
        let existing = workspace.terminals.count

        Task {
            await client.createTerminal(
                workspace: workspace.short,
                preset: "shell",
                title: "Terminal \(existing + 1)")
            expanded.insert(workspace.id)

            // Creation is a tmux window opening, so the record can lag the
            // call. Poll briefly rather than reading once and giving up —
            // reading once is why this silently did nothing.
            for _ in 0..<20 {
                await client.refresh()
                if let created = client.fleet.workspaces
                    .first(where: { $0.id == workspace.id })?
                    .terminals
                    .first(where: { !before.contains($0.id) })
                {
                    selection = .terminal(workspace: workspace.id, terminal: created.id)
                    return
                }
                try? await Task.sleep(for: .milliseconds(150))
            }
        }
    }

    private var currentWorkspace: Workspace? {
        switch selection {
        case .workspace(let id): return client.fleet.workspaces.first { $0.id == id }
        case .terminal(let id, _): return client.fleet.workspaces.first { $0.id == id }
        case nil: return client.fleet.workspaces.first
        }
    }

    private func step(by offset: Int) {
        let ordered = allTerminals
        guard !ordered.isEmpty else { return }
        let current = ordered.firstIndex { $0.id == selectedTerminal?.terminal.id } ?? 0
        // Wraps, because a list you can walk off the end of makes you look.
        let next = (current + offset + ordered.count) % ordered.count
        select(ordered[next])
    }

    /// Move the selection off a terminal that has gone.
    ///
    /// Prefers to stay where the user was looking: another terminal in the same
    /// workspace, whatever wants attention first, then anything running. Only
    /// falls back to the workspace itself when the workspace is empty.
    private func healSelection() {
        guard case .terminal(let workspaceID, let terminalID) = selection else { return }
        guard let workspace = client.fleet.workspaces.first(where: { $0.id == workspaceID })
        else {
            // The whole workspace went. Land on whatever is left rather than
            // on nothing.
            selection = client.fleet.workspaces.first.map { .workspace($0.id) }
            return
        }
        guard !workspace.terminals.contains(where: { $0.id == terminalID }) else { return }

        let candidates = workspace.terminals
        let next = candidates.first(where: { $0.status.wantsAttention })
            ?? candidates.first(where: { StateKind.parse($0.state) == .running })
            ?? candidates.first
        selection = next.map { .terminal(workspace: workspaceID, terminal: $0.id) }
            ?? .workspace(workspaceID)
    }

    private func selectTerminal(at index: Int) {
        let ordered = allTerminals
        guard index >= 0, index < ordered.count else { return }
        select(ordered[index])
    }

    private func select(_ terminal: Terminal) {
        guard
            let workspace = client.fleet.workspaces
                .first(where: { $0.terminals.contains(where: { $0.id == terminal.id }) })
        else { return }
        expanded.insert(workspace.id)
        selection = .terminal(workspace: workspace.id, terminal: terminal.id)
    }

    /// After closing one, land on the next terminal rather than on nothing.
    private func selectNeighbour(of terminal: Terminal) {
        let ordered = allTerminals
        guard let index = ordered.firstIndex(where: { $0.id == terminal.id }) else { return }
        let remaining = ordered.enumerated().filter { $0.offset != index }.map(\.element)
        if let next = remaining.first(where: { StateKind.parse($0.state) == .running })
            ?? remaining.first
        {
            select(next)
        } else {
            selection = currentWorkspace.map { .workspace($0.id) }
        }
    }
}

enum TerminalAction { case restart, dismissLost, stop }
