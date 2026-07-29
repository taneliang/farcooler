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
    @State private var newTerminalFor: Workspace?
    @State private var removeWorkspace: Workspace?

    /// What the detail pane is showing.
    enum Selection: Hashable {
        case workspace(String)
        case terminal(workspace: String, terminal: String)
    }

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            detail
        }
        .task {
            Notifier.shared.requestAuthorisation()
            await client.refresh()
            await client.refreshRepositories()
            await client.refreshRoots()
            expandAll()
            selectFirstRunningTerminal()
            // Pushed, not polled. The daemon derives once for every client and
            // sends only what changed, so a quiet fleet costs nothing and a
            // question from an agent arrives at once instead of up to a poll
            // interval later.
            client.startEvents()
        }
        .onDisappear { client.stopEvents() }
        .onCommand { command in run(command) }
        .onSelectIndex { index in selectTerminal(at: index) }
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
                expandAll()
            }
        }
        .sheet(item: $newTerminalFor) { ws in
            NewTerminalSheet(workspaceName: ws.task) { preset, title in
                await client.createTerminal(workspace: ws.short, preset: preset, title: title)
                expanded.insert(ws.id)
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

    private var sidebar: some View {
        VStack(spacing: 0) {
            sidebarHeader

            if client.fleet.workspaces.isEmpty {
                fleetPlaceholder
                Spacer(minLength: 0)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 3) {
                        ForEach(client.fleet.workspaces) { ws in
                            WorkspaceSection(
                                workspace: ws,
                                isExpanded: expanded.contains(ws.id),
                                selection: $selection,
                                onToggle: { toggle(ws.id) },
                                onNewTerminal: { newTerminalFor = ws },
                                onArchive: { Task { await client.archiveWorkspace(ws.short) } },
                                onRemove: { removeWorkspace = ws },
                                onTerminalAction: { term, action in
                                    Task { await run(action, on: term) }
                                }
                            )
                        }
                    }
                    // Matches the row margin, so a selected row's highlight
                    // sits a hair inside the sidebar rather than floating in a
                    // gutter wider than the indent it is supposed to show.
                    .padding(.horizontal, 8)
                    .padding(.vertical, 6)
                }
            }

            statusBar
        }
        .navigationSplitViewColumnWidth(min: 268, ideal: 300, max: 400)
    }

    /// Everything waiting on you, across every workspace.
    ///
    /// The one number the app exists to produce. It used to be reachable only
    /// by expanding each workspace and reading the rows — which is the work
    /// this screen is supposed to have already done for you.
    private var attentionCount: Int {
        client.fleet.workspaces
            .flatMap(\.terminals)
            .filter(\.status.wantsAttention)
            .count
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
            if let ws = workspace(wsID), let term = ws.terminals.first(where: { $0.id == termID }) {
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
            if let ws = workspace(wsID) {
                WorkspaceDetail(
                    workspace: ws,
                    onNewTerminal: { newTerminalFor = ws },
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

    private func expandAll() {
        expanded.formUnion(client.fleet.workspaces.map(\.id))
    }

    /// Land on something usable rather than an empty pane.
    private func selectFirstRunningTerminal() {
        guard selection == nil else { return }
        for ws in client.fleet.workspaces {
            if let t = ws.terminals.first(where: { StateKind.parse($0.state) == .running }) {
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

    private func run(_ command: AppCommand) {
        switch command {
        case .newTerminal:
            // Acts on whichever workspace you are in, so making a terminal is
            // one keystroke rather than a hunt for the right + button.
            if let workspace = currentWorkspace { newTerminalFor = workspace }

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

        case .newWorkspace: showNewWorkspace = true
        case .addRepository: showAddRepository = true
        case .reload: Task { await client.refresh() }
        case .showShortcuts: showShortcuts = true
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
