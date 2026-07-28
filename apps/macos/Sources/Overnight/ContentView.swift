import SwiftUI

struct ContentView: View {
    @StateObject private var client = DaemonClient()
    @StateObject private var service = ServiceRegistration()
    @State private var selection: Selection?
    @State private var expanded: Set<String> = []
    @State private var pollTask: Task<Void, Never>?

    @State private var showNewWorkspace = false
    @State private var showAddRepository = false
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
            await client.refresh()
            await client.refreshRepositories()
            await client.refreshRoots()
            expandAll()
            selectFirstRunningTerminal()
            startPolling()
        }
        .onDisappear { pollTask?.cancel() }
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
                emptyFleet
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
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                }
            }

            statusBar
        }
        .navigationSplitViewColumnWidth(min: 268, ideal: 300, max: 400)
    }

    private var sidebarHeader: some View {
        HStack(spacing: 8) {
            Text("Fleet").font(.headline)
            if client.busy { ProgressView().controlSize(.mini) }
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
        .padding(.horizontal, 16)
        .padding(.top, 14)
        .padding(.bottom, 10)
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
    /// Surfaced here rather than buried in a settings pane because it decides
    /// whether this host is reachable when nobody is sitting at it, which is
    /// the difference between a terminal multiplexer and the thing this is
    /// meant to be.
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

                loginItemToggle

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

    private func startPolling() {
        pollTask?.cancel()
        pollTask = Task {
            while !Task.isCancelled {
                // The screen streams, so only the fleet list needs polling and it
                // is not latency sensitive.
                try? await Task.sleep(for: .seconds(3))
                if Task.isCancelled { return }
                await client.refresh()
            }
        }
    }
}

enum TerminalAction { case restart, dismissLost, stop }
