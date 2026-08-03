import AgentKit
import AppKit
import SwiftUI

struct ContentView: View {
    @StateObject private var client = DaemonClient()
    @ObservedObject private var preferences = Preferences.shared
    @State private var selection: Selection?
    @State private var expanded: Set<String> = []

    @State private var showNewWorkspace = false
    /// Which repository the new-workspace sheet should open on, when it was
    /// reached from a project header rather than the sidebar's own `+`.
    @State private var newWorkspaceRepository = ""
    @State private var showAddRepository = false
    @State private var showShortcuts = false
    @State private var showAbout = false
    @State private var query = ""
    @State private var showQuickCreate = false
    @AppStorage("tasks.lastProject") private var lastProject = ""
    /// The terminal that was open when the app last closed, as
    /// `workspace/terminal`.
    ///
    /// Reopening somewhere else is a small thing that costs a real one: the
    /// pane you were reading is the reason you came back, and finding it again
    /// means walking a sidebar you had already navigated once.
    @AppStorage("fleet.lastTerminal") private var lastTerminal = ""
    @FocusState private var searchFocused: Bool
    @State private var removeWorkspace: Workspace?
    @State private var showResumeBranch = false
    @State private var showPalette = false
    /// Quick-create's draft, reachable from here so that what was typed into
    /// the palette arrives in the panel that acts on it. See `perform`.
    @AppStorage("tasks.draft") private var taskDraft = ""
    @State private var showImportWorktrees = false
    /// One divider resize at a time. See `resizeDivider`.
    @State private var resizingDivider = false
    /// Set when `setPaneMode` comes back `confirmationRequired` — a turn is in
    /// flight and switching would cancel it. Drives a sheet the same way
    /// `removeWorkspace` does, rather than a banner: this refusal has an
    /// answer ("cancel it anyway?") a banner cannot offer.
    @State private var pendingPaneModeSwitch: PaneModeConfirmation?

    /// What the detail pane is showing.
    enum Selection: Hashable {
        case workspace(String)
        case terminal(workspace: String, terminal: String)
    }

    /// What confirming a pane-mode switch would do, and to which pane.
    struct PaneModeConfirmation: Identifiable {
        let id = UUID()
        let terminal: String
        let mode: String
        let message: String
    }

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            detail
        }
        .task {
            Notifier.shared.requestAuthorisation()
            PushRegistration.shared.label = { Host.current().localizedName ?? "Mac" }
            AccountSection.afterSignIn = { await PushRegistration.shared.sendIfPossible() }
            // Before the first read, not after: a stale daemon left over from an
            // earlier build would otherwise answer it, and everything from that
            // point on would be this app talking to a different program.
            if let problem = await LocalDaemon.shared.ensure().problem {
                client.lastError = problem
            }
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
        // Recorded as it changes rather than on quit: an app that is force
        // quit, crashes, or is killed by a rebuild never gets a last word, and
        // this is exactly the state worth surviving all three.
        .onChange(of: selection) { _, now in
            if case let .terminal(workspace, terminal) = now {
                lastTerminal = "\(workspace)/\(terminal)"
            }
        }
        // The event stream is a long-lived process pointed at a machine when
        // it starts, so changing which machine this window drives has to
        // restart it rather than let it go on listening to the old one.
        .onChange(of: preferences.remoteHost) { _, _ in Task { await client.hostChanged() } }
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
            // Selecting a pane focuses it, which is also what switches to its
            // layout. Done here rather than in `detail`, because it is a write and
            // a view must not perform one while it is being evaluated.
            //
            // One call, not two. It used to activate the group and stop there,
            // which put the layout on screen and left its REMEMBERED focus in
            // charge — so clicking Terminal 6 landed you on whichever pane of that
            // layout you had used last. `⌘P` had the same fault for the same
            // reason: both set a selection and let the layout overrule it.
            if case .terminal(let wsID, let termID) = new,
                let workspace = client.fleet.workspaces.first(where: { $0.id == wsID }),
                let holder = client.group(holding: termID, in: wsID),
                let pane = holder.pane(termID),
                !holder.isActive || !pane.focused
            {
                Task { await client.focusPane(pane.short, in: workspace) }
            }

            // Opening a terminal is what ends `done`. Being listed is not being
            // read, so this is deliberately tied to selection.
            if case .terminal(_, let id) = new,
                let terminal = allTerminals.first(where: { $0.id == id })
            {
                // Stamped here rather than in the palette, so every way of
                // arriving counts: a sidebar click, ⌘], ⌃B o, a jump from the
                // palette itself. A switcher that only learned from its own
                // choices would order by where you had used IT, not by where you
                // have been.
                VisitLog.shared.visited(id)
                Task { await client.markSeen(terminal.short) }
            }
        }
        .sheet(isPresented: $showShortcuts) { ShortcutsSheet() }
        .sheet(isPresented: $showAbout) { AboutSheet() }
        .sheet(isPresented: $showImportWorktrees) {
            ImportWorktrees(
                projects: client.repositories,
                project: $lastProject,
                load: { await client.existingWorktrees(project: $0) },
                onImport: { worktrees, project in
                    await client.importWorktrees(worktrees, project: project)
                }
            )
        }
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
        // Centred and over everything, unlike quick-create. This one is not
        // something you work alongside — it is a switcher, it is on screen for
        // about a second, and while it is there every keystroke belongs to it.
        // The scrim is what makes that true for the mouse as well: a panel this
        // large with a live terminal showing round the edges invites a click
        // that lands somewhere surprising.
        .overlay {
            if showPalette {
                ZStack {
                    Color.black.opacity(0.12)
                        .ignoresSafeArea()
                        .onTapGesture { showPalette = false }
                    CommandPalette(
                        workspaces: client.fleet.workspaces,
                        current: selection,
                        screen: { await client.screen(terminal: $0) },
                        onRun: { perform($0) },
                        onClose: { showPalette = false }
                    )
                }
                .transition(.opacity)
            }
        }
        .animation(.snappy(duration: 0.14), value: showPalette)
        .onReceive(
            NotificationCenter.default.publisher(
                for: NSApplication.didBecomeActiveNotification)
        ) { _ in
            // The user may have approved the login item in System Settings
            // while we were in the background; nothing tells us but this.
        }
        .sheet(isPresented: $showAddRepository) {
            AddRepositorySheet(
                roots: client.roots,
                onAddRoot: { path in
                    let failure = await client.addRoot(path)
                    await client.refreshRoots()
                    return failure
                },
                onRegister: { path in await client.registerRepository(path) },
                onRegistered: {
                    Task {
                        await client.refreshRepositories()
                        // Only if there is something to offer. A sheet that opens
                        // to say "nothing to import" is worse than no sheet.
                        if let newest = client.repositories.last,
                            !(await client.existingWorktrees(project: newest.id)).isEmpty
                        {
                            lastProject = newest.id
                            showImportWorktrees = true
                        }
                    }
                }
            )
        }
        .sheet(isPresented: $showNewWorkspace) {
            NewWorkspaceSheet(
                repositories: client.repositories,
                preselected: newWorkspaceRepository
            ) { repo, task, branch, base in
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
        .sheet(item: $pendingPaneModeSwitch) { pending in
            PaneModeConfirmSheet(message: pending.message) {
                await client.setPaneMode(pending.terminal, mode: pending.mode, force: true)
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

    /// Start a worktree in a named project.
    ///
    /// The sheet already has a repository picker; this just answers it in
    /// advance, because someone clicking `+` on a project header has already
    /// said which one.
    private func newWorktree(in project: String) {
        newWorkspaceRepository = repositoryName(from: project)
        showNewWorkspace = true
    }

    /// A terminal in the repository's own checkout.
    private func newMainTerminal(in project: String) async {
        guard let workspace = await client.mainWorkspace(repo: repositoryName(from: project))
        else { return }
        await client.createTerminal(workspace: workspace, preset: "shell", title: "")
        await client.refresh()
    }

    /// A group's key carries the host when a fleet spans machines
    /// (`project · host`); the repository is the part before it.
    private func repositoryName(from project: String) -> String {
        project.components(separatedBy: " · ").first ?? project
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
                            ProjectHeader(
                                name: project,
                                count: workspaces.count,
                                onNewWorktree: { newWorktree(in: project) },
                                onNewTerminal: { Task { await newMainTerminal(in: project) } }
                            )
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
                                    layouts: client.layouts[ws.id] ?? [],
                                    onMoveToLayout: { term, group in
                                        moveToLayout(term, in: ws, group: group)
                                    },
                                    onDropTogether: { dragged, onto in
                                        placePane(dragged, onto: onto.id, side: .right, in: ws)
                                    },
                                    tiled: Set(client.activeGroup(ws.id)?.terminals ?? [])
                                )
                            }
                        }
                    }
                    .padding(.bottom, 10)
                }
            }

            statusBar
        }
        // Declared in exactly one place. A second declaration on the
        // `NavigationSplitView`'s sidebar closure made which width the column
        // settled on nondeterministic, and the window drifted with it.
        .navigationSplitViewColumnWidth(min: 268, ideal: 320, max: 440)
    }

    /// Search, because worktrees are unbounded.
    ///
    /// It matches terminals too, so typing an agent's name finds the worktree
    /// containing it — which is how you reach an agent on another machine
    /// without going looking for the machine.
    private var searchField: some View {
        SidebarRow {
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

        }
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
        SidebarRow {
            HStack(spacing: 8) {
            // The machine, not the word "Fleet". This pane IS the fleet — the
            // thing it could not tell you was whose.
            MachinePicker()
            if client.busy { ProgressView().controlSize(.mini) }

            Spacer()

            // With the actions, not beside the title. It is a button — it jumps
            // to the next agent waiting — and sitting it against the machine
            // name read as part of the name, which is the one thing it is not.
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
            // A menu rather than a button, because "add a repository" has to be
            // reachable at all times. It used to live only in the empty state,
            // so once you had one workspace there was no way to add a second
            // repository without dropping to the terminal.
            SidebarMenuButton(
                systemImage: "plus",
                help: "Add a workspace, a repository, or a machine",
                items: [
                    SidebarMenuItem(title: "New workspace…") {
                        newWorkspaceRepository = ""
                        showNewWorkspace = true
                    },
                    SidebarMenuItem(title: "Import existing worktrees…") { openImport() },
                    .separator,
                    SidebarMenuItem(title: "Add repository…") { showAddRepository = true },
                    // Here as well as in the picker, because this is the menu
                    // people open looking for "add a thing" — and a machine is
                    // a thing you add.
                    SidebarMenuItem(title: "Add a machine…") {
                        Preferences.shared.settingsTab = "machines"
                        NSApp.sendAction(
                            Selector(("showSettingsWindow:")), to: nil, from: nil)
                    },
                ])
            }
        }
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
            // Every terminal is in a layout now — it IS a tmux window, and a
            // window IS a layout — so the second branch is reached only in the
            // seconds before the first `layout show` comes back, and after that
            // never. It is kept because "we have not read the layouts yet" and
            // "this terminal is in no layout" look identical from here, and
            // showing the terminal is the right answer to both.
            //
            // The condition deliberately asks whether the terminal is in ANY
            // layout rather than in the ACTIVE one. It used to ask the latter, so
            // selecting a pane belonging to a different layout showed it on its
            // own and the view bounced between arrangement and single terminal as
            // you clicked down the sidebar.
            if let ws = workspace(wsID), client.group(holding: termID, in: wsID) != nil,
                let group = client.activeGroup(wsID)
            {
                tiled(ws, group: group)
            } else if let ws = workspace(wsID),
                let term = ws.terminals.first(where: { $0.id == termID })
            {
                TerminalPane(
                    terminal: term,
                    workspace: ws,
                    binary: client.cliPath,
                    environment: client.cliEnvironment,
                    hostArguments: client.cliHostArguments,
                    onGeometry: { cols, rows in
                        await client.resize(terminal: term.short, columns: cols, rows: rows)
                    },
                    onSearchFiles: { query in await client.searchFiles(in: ws, query: query) },
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
                tiled(ws, group: group)
            } else if let ws = workspace(wsID) {
                WorkspaceDetail(
                    workspace: ws,
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

    /// The layout, wired up.
    ///
    /// One builder for both selections. Selecting a pane and selecting the
    /// worktree it is in put the same view on screen with the same six callbacks,
    /// and while they were written out twice they drifted: the drag handler was
    /// fixed in one copy and not the other, so dropping a pane behaved differently
    /// depending on which sidebar row you had clicked last.
    private func tiled(_ ws: Workspace, group: PaneGroup) -> some View {
        TileView(
            groups: client.layouts[ws.id] ?? [group],
            workspace: ws,
            binary: client.cliPath,
            environment: client.cliEnvironment,
            hostArguments: client.cliHostArguments,
            onFocus: { id in
                selection = .terminal(workspace: ws.id, terminal: id)
                guard let pane = client.group(holding: id, in: ws.id)?.pane(id) else { return }
                Task { await client.focusPane(pane.short, in: ws) }
            },
            onSelectGroup: { chosen in
                Task {
                    // Land in the layout you just chose, not on whatever pane the
                    // previous one had focused.
                    reveal(await client.selectLayout(chosen.id, in: ws), in: ws)
                }
            },
            onDropOnPane: { dragged, target, side in
                placePane(dragged, onto: target, side: side, in: ws)
            },
            onViewport: { columns, rows in
                await client.viewport(columns: columns, rows: rows, in: ws)
            },
            onResizeDivider: { terminal, side, cells in
                resizeDivider(terminal, side: side, cells: cells, in: ws)
            },
            onSearchFiles: { query in await client.searchFiles(in: ws, query: query) },
            onSwitchPaneMode: { terminal in Task { await togglePaneMode(terminal) } }
        )
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
        case .dismissLost: await client.dismissLost(term)
        case .stop: await client.stop(terminal: term.short)
        }
    }

    private func workspace(_ id: String) -> Workspace? {
        client.fleet.workspaces.first { $0.id == id }
    }

    private func toggle(_ id: String) {
        if expanded.contains(id) { expanded.remove(id) } else { expanded.insert(id) }
    }

    /// Land on something usable rather than an empty pane.
    /// Where to land on launch.
    ///
    /// Whatever wants you first, wherever it is — including on another machine.
    /// Falling back to "the first running terminal" would open a fleet on
    /// something arbitrary while an agent two projects down waits for an answer.
    private func selectFirstRunningTerminal() {
        guard selection == nil else { return }

        // Where you left off, if it is still there. A terminal that has since
        // exited falls through to the rules below rather than selecting
        // nothing — the saved id is a preference, not a promise.
        let saved = lastTerminal.split(separator: "/", maxSplits: 1).map(String.init)
        if saved.count == 2,
            let ws = client.fleet.workspaces.first(where: { $0.id == saved[0] }),
            ws.terminals.contains(where: { $0.id == saved[1] })
        {
            expanded.insert(ws.id)
            selection = .terminal(workspace: ws.id, terminal: saved[1])
            return
        }

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
    /// Every one of them is a single `layout` call whose reply is the workspace's
    /// whole layout, so there is nothing to reconcile here: tmux decides what a
    /// zoom or a split means, and it decides the same way for this app, for the
    /// CLI and for an agent driving the CLI.
    ///
    /// This file no longer contributes geometry. Directional focus used to be
    /// worked out here from a recomputed arrangement; it is now read off the
    /// rectangles tmux reported, which is the only copy.
    private func tile(_ command: TileCommand) async {
        guard let workspace = tileTarget else { return }
        let group = client.activeGroup(workspace.id)
        /// The pane a keystroke acts on: the selected one, else whatever tmux says
        /// is focused.
        let here: PaneRect? = {
            if case .terminal(_, let id) = selection, let pane = group?.pane(id) { return pane }
            return group?.panes.first(where: \.focused)
        }()

        switch command {
        case .zoom:
            await client.zoomPane(nil, in: workspace)

        case .focusNext:
            await client.focusPane(step: "--next", in: workspace)
        case .focusPrevious:
            await client.focusPane(step: "--prev", in: workspace)

        case .focus(let direction):
            guard let group, let from = here,
                let next = group.neighbour(of: from.id, direction)
            else { return }
            await client.focusPane(next.short, in: workspace)

        case .focusIndex(let n):
            await client.focusPane(number: n, in: workspace)

        case .cycle:
            await client.cycleLayout(workspace)

        case .preset(let preset):
            await client.applyPreset(preset, in: workspace)

        case .splitRight, .splitDown:
            // One call. It used to be create-then-join-then-apply-a-preset, three
            // round trips whose only way of saying WHERE the new pane went was to
            // re-arrange every pane in the layout — so splitting the third pane of
            // four rebuilt the other three as well. `layout split` splits the pane
            // you name, on the side you name, and leaves the rest alone.
            let side: TileDirection = command == .splitRight ? .right : .bottom
            let groups = await client.split(workspace, beside: here?.short, side: side)
            // Land in the pane that was just made, which is the one tmux focuses.
            reveal(groups, in: workspace)

        case .breakPane:
            guard let here else { return }
            reveal(
                await client.breakPane(here.short, in: workspace),
                in: workspace, preferring: here.id)

        case .closePane:
            // tmux's `x`, and it means the same thing: the pane's process ends.
            // Routed through the existing close so there is one implementation of
            // what closing a terminal does.
            run(.closeTerminal)

        case .newGroup:
            await openTerminalInNewLayout(workspace)

        case .nextGroup:
            // Landing in the new layout is the point of switching to it. Without
            // this the selection stayed on a pane from the OLD layout, which the
            // detail view then showed on its own — so ⌃B n looked like it opened a
            // random terminal and came back.
            reveal(await client.selectLayout("--next", in: workspace), in: workspace)
        case .previousGroup:
            reveal(await client.selectLayout("--prev", in: workspace), in: workspace)

        case .toggleAgentPane:
            // Falls back to the plain selection when there is no tmux group
            // yet — the few seconds `detail`'s own comment describes, before
            // the first `layout show` has come back, where `here` is nil but
            // a terminal is still very much selected.
            let target =
                here.flatMap { rect in workspace.terminals.first { $0.id == rect.id } }
                ?? selectedTerminal?.terminal
                // Selecting a LAYOUT TAB is not selecting a terminal, and a
                // cached layout does not always mark a pane focused — so both
                // of the above are nil for the commonest way of getting here,
                // and the command silently did nothing at all. A layout with
                // one pane has no ambiguity about which pane is meant.
                ?? group?.panes.first.flatMap { pane in
                    workspace.terminals.first { $0.id == pane.id }
                }
            guard let target else {
                // Never silent. A keystroke that does nothing and says nothing
                // is indistinguishable from a broken feature.
                client.lastError = "No pane to switch — select a terminal first."
                return
            }
            // Said here rather than left to the daemon's refusal, because this
            // is where the agent's NAME is known. The on-pane button is hidden
            // for a pane that cannot switch; the keystroke was not, so ⌃B a on
            // a Codex pane did nothing and explained nothing.
            guard target.canSwitchPaneMode || target.isAgentPane else {
                let agent = target.agentLabel
                client.lastError = "\(agent) has no chat view. Only Claude renders as a chat."
                return
            }
            await togglePaneMode(target)

        case .help:
            showShortcuts = true
        }
    }

    /// Ask the daemon to flip a pane between its terminal and its agent chat.
    ///
    /// One call, and the daemon is the one deciding whether that is even
    /// possible — a client guessing "this preset can't be an agent" would be
    /// exactly the kind of state the design says clients never derive.
    private func togglePaneMode(_ terminal: Terminal) async {
        let target = terminal.isAgentPane ? "terminal" : "agent"
        switch await client.setPaneMode(terminal.short, mode: target) {
        case .ok, .failed:
            // A failure already reached `client.lastError`, which the banner
            // already shows — nothing further to do from here.
            break
        case let .confirmationRequired(message):
            pendingPaneModeSwitch = PaneModeConfirmation(
                terminal: terminal.short, mode: target, message: message)
        }
    }

    /// Send a terminal to another layout, or to one of its own.
    ///
    /// `nil` is `break-pane`: a layout with just this in it. Naming a layout moves
    /// the pane against that layout's focused pane, because "which layout" is only
    /// half an instruction — tmux has to be told which pane and which edge, and the
    /// menu has no way to ask. Dragging is how you say the other half, and the drop
    /// indicator is why that is easier than answering a dialog about it.
    private func moveToLayout(_ terminal: Terminal, in workspace: Workspace, group: PaneGroup?) {
        Task {
            let groups: [PaneGroup]
            if let group, let onto = group.panes.first(where: \.focused) ?? group.panes.first {
                groups = await client.movePane(
                    terminal.short, onto: onto.short, side: .right, in: workspace)
            } else {
                groups = await client.breakPane(terminal.short, in: workspace)
            }
            reveal(groups, in: workspace, preferring: terminal.id)
        }
    }

    /// Open the import sheet against a project that exists.
    private func openImport() {
        if lastProject.isEmpty || !client.repositories.contains(where: { $0.id == lastProject }) {
            lastProject = client.repositories.first?.id ?? ""
        }
        showImportWorktrees = true
    }

    /// Drop a terminal on an edge of a pane: it splits that pane on that edge.
    ///
    /// One write for every drag in the app — a pane onto a pane, a sidebar row
    /// onto a pane, a sidebar row onto another row — because they all say the same
    /// thing: this terminal, against that one, on this side. It was three
    /// operations while a layout was an ordered list, and the list could only
    /// express "before" and "after", which is why dropping on the left half of a
    /// pane and on its right half used to do the same thing.
    ///
    /// Move a divider, in cells. Answers whether the request was taken.
    ///
    /// Serialised rather than queued. A drag produces one of these per cell
    /// crossed and each is a round trip, so a fast drag would stack up dozens of
    /// requests that land after the pointer has stopped and walk the divider past
    /// where it was let go.
    ///
    /// Refusing has to be VISIBLE to the caller, which is what the return value is
    /// for. The handle counts cells the layout has actually moved by, so a refusal
    /// leaves them owed and the next mouse event asks for them again. Without
    /// that the handle counted a dropped request as done and threw its cells away
    /// — losing most of them over a fast drag, so the divider followed the pointer
    /// at a fraction of its speed.
    @discardableResult
    private func resizeDivider(
        _ terminal: String, side: TileDirection, cells: Int, in workspace: Workspace
    ) -> Bool {
        guard cells != 0, !resizingDivider else { return false }
        guard let pane = client.group(holding: terminal, in: workspace.id)?.pane(terminal) else {
            return false
        }
        resizingDivider = true
        Task {
            await client.resizePane(pane.short, side: side, cells: cells, in: workspace)
            resizingDivider = false
        }
        return true
    }

    /// Works across layouts too: the pane leaves whichever one it was in.
    private func placePane(
        _ dragged: String, onto target: String, side: TileDirection, in workspace: Workspace
    ) {
        let shorts = [dragged, target].compactMap { id in
            workspace.terminals.first { $0.id == id }?.short
        }
        guard shorts.count == 2 else { return }
        Task {
            reveal(
                await client.movePane(shorts[0], onto: shorts[1], side: side, in: workspace),
                in: workspace, preferring: dragged)
        }
    }

    /// Select the active layout's focused pane after a layout command.
    ///
    /// Every command that changes which group is on screen goes through this, so
    /// "the thing I am looking at" and "the thing the layout says is focused" cannot
    /// disagree. `preferring` is for the cases where the command was about a
    /// specific terminal and that terminal should win.
    private func reveal(
        _ groups: [PaneGroup], in workspace: Workspace, preferring: String? = nil
    ) {
        guard let active = groups.first(where: { $0.isActive }) ?? groups.first else {
            // No layouts left, which now means no terminals left. Fall back to
            // the worktree rather than to a pane that no longer exists.
            selection = .workspace(workspace.id)
            return
        }
        let target = preferring.flatMap { active.terminals.contains($0) ? $0 : nil }
            ?? active.focused
            ?? active.terminals.first
        guard let target else {
            selection = .workspace(workspace.id)
            return
        }
        selection = .terminal(workspace: workspace.id, terminal: target)
    }

    /// Follow the layout's focus when something else moved it.
    ///
    /// The CLI and an agent can both focus a pane, and when they do the app has
    /// to be looking at it — otherwise `farcooler layout focus` from a script
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
        case .about: showAbout = true
        case .search: searchFocused = true

        // Toggles rather than opens. ⌘P on an open palette is what a hand
        // reaches for when it changed its mind, and every switcher on this
        // machine closes that way.
        case .commandPalette: showPalette.toggle()

        case .toggleSidebar:
            // See `Sidebar.toggle`: AppKit's own action, with the collapse
            // behaviour set first so the detail pane absorbs the space instead of
            // the window growing.
            Sidebar.toggle()
        }
    }

    /// Carry out whatever was chosen in the palette.
    ///
    /// Every case here routes into a method that already existed, and that is
    /// the whole design of `PaletteAction`: the palette knows what you picked
    /// and nothing about what picking it means, so opening a terminal from the
    /// panel and clicking it in the sidebar cannot drift apart.
    private func perform(_ action: PaletteAction) {
        showPalette = false
        switch action {
        case .openTerminal(let workspace, let terminal):
            expanded.insert(workspace)
            selection = .terminal(workspace: workspace, terminal: terminal)

        case .openWorkspace(let workspace):
            expanded.insert(workspace)
            selection = .workspace(workspace)

        case .newTerminal(let id):
            guard let workspace = client.fleet.workspaces.first(where: { $0.id == id }) else {
                return
            }
            newTerminal(in: workspace)

        case .newTask(let described):
            if client.repositories.isEmpty {
                showAddRepository = true
                return
            }
            if lastProject.isEmpty { lastProject = client.repositories[0].id }
            // What was typed into the palette carries over as the description,
            // because in this panel it nearly always was one. It never
            // overwrites a draft already in progress — that draft is often the
            // thing someone opened the palette to go and look something up for.
            if !described.isEmpty, taskDraft.isEmpty { taskDraft = described }
            showQuickCreate = true

        case .togglePaneMode(let workspace, let terminal):
            guard
                let target = client.fleet.workspaces.first(where: { $0.id == workspace })?
                    .terminals.first(where: { $0.id == terminal })
            else { return }
            Task { await togglePaneMode(target) }
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
        Task { await openTerminalInNewLayout(workspace) }
    }

    /// A new terminal, in a layout of its own.
    ///
    /// Every way of making a terminal goes through this — the sidebar button, ⌘T,
    /// the palette's action, ⌃B c.
    ///
    /// One call now, where it used to be two. A terminal IS a tmux window and a
    /// window IS a layout, so creating one already produces the layout; the
    /// separate "make a group, then put it in the group" step was describing a
    /// distinction that no longer exists.
    ///
    /// tmux's `c` opens a window with a shell in it. So does this.
    @discardableResult
    private func openTerminalInNewLayout(_ workspace: Workspace) async -> Terminal? {
        expanded.insert(workspace.id)
        guard
            let created = await client.createTerminal(
                in: workspace,
                preset: "shell",
                title: "Terminal \(workspace.terminals.count + 1)")
        else { return nil }
        selection = .terminal(workspace: workspace.id, terminal: created.id)
        return created
    }

    /// The workspace the selection is in, or the first one.
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
