import SwiftUI

struct ContentView: View {
    @StateObject private var client = DaemonClient()
    @State private var selectedWorkspace: Workspace?
    @State private var selectedTerminal: Terminal?
    @State private var output = ""
    @State private var pollTask: Task<Void, Never>?
    @State private var ticks = 0

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            detail
        }
        .task {
            await client.refresh()
            selectFirstRunningTerminal()
            startPolling()
        }
        .onDisappear { pollTask?.cancel() }
    }

    // MARK: - Sidebar: the fleet

    private var sidebar: some View {
        List(selection: $selectedWorkspace) {
            Section {
                ForEach(client.fleet.workspaces) { ws in
                    WorkspaceRow(workspace: ws, selectedTerminal: $selectedTerminal)
                        .tag(ws)
                }
            } header: {
                HStack {
                    Text("Fleet")
                    Spacer()
                    if client.busy { ProgressView().controlSize(.mini) }
                }
            }

            if !client.fleet.runtimeHealthy {
                // An unusable inventory never claims life: everything derives lost.
                Label(
                    "tmux runtime unavailable, every terminal derives lost",
                    systemImage: "exclamationmark.triangle.fill"
                )
                .foregroundStyle(.orange)
                .font(.caption)
            }
        }
        .listStyle(.sidebar)
        .navigationSplitViewColumnWidth(min: 260, ideal: 300)
        .safeAreaInset(edge: .bottom) { statusBar }
    }

    private var statusBar: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(client.fleet.runtimeHealthy ? Color.green : Color.orange)
                .frame(width: 7, height: 7)
            Text("\(client.fleet.livePanes) live panes")
                .font(.caption)
                .foregroundStyle(.secondary)
            Spacer()
            Button {
                Task { await client.refresh() }
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.borderless)
            .help("Refresh")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
    }

    // MARK: - Detail: the terminal

    @ViewBuilder
    private var detail: some View {
        if let terminal = selectedTerminal {
            TerminalPane(
                terminal: terminal,
                screen: $output,
                onBytes: { bytes in
                    // Straight into the serial queue. Never spawn a task per
                    // keystroke: concurrent sends reorder the bytes and "echo"
                    // arrives as "ehco".
                    let q = inputQueue(for: terminal.short)
                    Task { await q.submit(bytes) }
                },
                onGeometry: { cols, rows in
                    Task { await client.resize(terminal: terminal.short, columns: cols, rows: rows) }
                },
                onRestart: { Task { await client.restart(terminal: terminal.short) } },
                onDismiss: { Task { await client.dismissLost(terminal: terminal.short) } },
                onStop: { Task { await client.stop(terminal: terminal.short) } }
            )
            .task(id: terminal.id) {
                output = await client.screen(terminal: terminal.short)
            }
        } else if let ws = selectedWorkspace {
            WorkspaceSummary(workspace: ws) { preset in
                Task { await client.createTerminal(workspace: ws.short, preset: preset) }
            }
        } else {
            ContentUnavailableView(
                "Select a workspace",
                systemImage: "rectangle.split.3x1",
                description: Text("Each workspace is one worktree and branch for one task.")
            )
        }
    }

    /// Land on something usable rather than an empty pane.
    ///
    /// A running terminal is what you almost always want on open, so prefer one,
    /// then fall back to any terminal, then to a workspace.
    private func selectFirstRunningTerminal() {
        guard selectedTerminal == nil else { return }

        for ws in client.fleet.workspaces {
            if let t = ws.terminals.first(where: { StateKind.parse($0.state) == .running }) {
                selectedWorkspace = ws
                selectedTerminal = t
                return
            }
        }
        for ws in client.fleet.workspaces where !ws.terminals.isEmpty {
            selectedWorkspace = ws
            selectedTerminal = ws.terminals.first
            return
        }
        selectedWorkspace = client.fleet.workspaces.first
    }

    /// One serial input queue per terminal, created on first keystroke.
    ///
    /// Held outside SwiftUI state because recreating it on a view update would
    /// lose the ordering guarantee it exists to provide.
    private func inputQueue(for terminal: String) -> InputQueue {
        if let existing = Self.queues[terminal] { return existing }

        let client = self.client
        let q = InputQueue { bytes in
            await client.sendBytes(terminal: terminal, bytes: bytes)
        }
        Self.queues[terminal] = q
        return q
    }

    private static var queues: [String: InputQueue] = [:]

    private func startPolling() {
        pollTask?.cancel()
        pollTask = Task {
            while !Task.isCancelled {
                // Two cadences: the screen refreshes fast enough to feel live,
                // the fleet list far less often because it is not latency
                // sensitive and each refresh costs a tmux inventory.
                try? await Task.sleep(for: .milliseconds(60))
                if Task.isCancelled { return }

                if let t = selectedTerminal {
                    output = await client.screen(terminal: t.short)
                }

                ticks += 1
                if ticks % 50 == 0 { await client.refresh() }
            }
        }
    }
}

// MARK: - Rows

struct WorkspaceRow: View {
    let workspace: Workspace
    @Binding var selectedTerminal: Terminal?

    var body: some View {
        DisclosureGroup {
            ForEach(workspace.terminals) { t in
                Button {
                    selectedTerminal = t
                } label: {
                    HStack(spacing: 8) {
                        StateDot(state: t.state)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(t.title).font(.callout)
                            Text(t.preset)
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        if t.epoch > 0 {
                            Text("e\(t.epoch)")
                                .font(.caption2.monospaced())
                                .foregroundStyle(.tertiary)
                        }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .background(
                    selectedTerminal?.id == t.id
                        ? Color.accentColor.opacity(0.15) : Color.clear
                )
                .clipShape(RoundedRectangle(cornerRadius: 5))
            }
        } label: {
            HStack(spacing: 8) {
                StateDot(state: workspace.state)
                VStack(alignment: .leading, spacing: 1) {
                    Text(workspace.task).font(.body)
                    Text(workspace.branch)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
    }
}

struct StateDot: View {
    let state: String

    private var color: Color {
        switch StateKind.parse(state) {
        case .running, .active: return .green
        case .starting: return .yellow
        case .exited, .ready: return .secondary
        case .lost, .error: return .red
        case .archived: return .gray
        case .unknown: return .gray
        }
    }

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 8, height: 8)
            .help(state)
    }
}

// MARK: - Panes

struct WorkspaceSummary: View {
    let workspace: Workspace
    let onCreate: (String) -> Void

    private let presets = ["shell", "claude", "codex", "cursor"]

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 4) {
                Text(workspace.task).font(.largeTitle.weight(.semibold))
                Text(workspace.branch).font(.title3).foregroundStyle(.secondary)
            }

            LabeledContent("State") {
                HStack(spacing: 6) {
                    StateDot(state: workspace.state)
                    Text(workspace.state)
                }
            }
            LabeledContent("Worktree") {
                Text(workspace.worktree).font(.caption.monospaced()).textSelection(.enabled)
            }
            LabeledContent("Terminals") { Text("\(workspace.terminals.count)") }

            Divider()

            Text("Launch a terminal").font(.headline)
            HStack {
                ForEach(presets, id: \.self) { p in
                    Button(p) { onCreate(p) }
                }
            }

            Spacer()
        }
        .padding(28)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct TerminalPane: View {
    let terminal: Terminal
    @Binding var screen: String
    let onBytes: ([UInt8]) -> Void
    let onGeometry: (Int, Int) -> Void
    let onRestart: () -> Void
    let onDismiss: () -> Void
    let onStop: () -> Void

    private var kind: StateKind { StateKind.parse(terminal.state) }

    var body: some View {
        VStack(spacing: 0) {
            header

            if kind == .running || kind == .starting {
                // Keystrokes go straight here. No input box: a text field would
                // steal Return and swallow the control chords an agent needs.
                TerminalSurface(
                    screen: screen,
                    onBytes: onBytes,
                    onGeometry: onGeometry
                )
            } else {
                inactiveScreen
            }

            footer
        }
    }

    private var inactiveScreen: some View {
        ScrollView {
            Text(screen.isEmpty ? "(no output)" : screen)
                .font(.system(.body, design: .monospaced))
                .textSelection(.enabled)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
        }
        .background(Color(nsColor: .textBackgroundColor))
    }

    private var header: some View {
        HStack(spacing: 10) {
            StateDot(state: terminal.state)
            Text(terminal.title).font(.headline)
            Text(terminal.preset).font(.caption).foregroundStyle(.secondary)

            Spacer()

            if kind == .lost {
                // A lost terminal is truthfully lost. Dismissing acknowledges it
                // without ever relabelling it as an observed exit.
                Button("Dismiss loss", action: onDismiss)
                Button("Restart", action: onRestart).buttonStyle(.borderedProminent)
            } else if kind == .exited {
                Button("Restart", action: onRestart)
            } else if kind == .running {
                Button("Stop", action: onStop)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.bar)
    }

    @ViewBuilder
    private var footer: some View {
        Divider()
        if kind == .running || kind == .starting {
            HStack(spacing: 6) {
                Image(systemName: "keyboard")
                Text("Typing goes straight to the terminal. Ctrl-C, arrows, Tab and Esc all pass through.")
                Spacer()
                Text("\(terminal.preset)")
                    .foregroundStyle(.tertiary)
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
        } else {
            Text(explanation)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
        }
    }

    private var explanation: String {
        switch kind {
        case .lost:
            return
                "This terminal was expected to be running but no live tagged pane proves it. "
                + "Overnight will not guess: it says lost rather than claiming an exit."
        case .exited:
            return "The command exited and Overnight observed it. Restart begins a new epoch."
        case .starting:
            return "Coming up. Not yet proved by a live pane."
        case .error:
            return "Creation never established a live runtime."
        default:
            return "Not accepting input."
        }
    }
}
