import SwiftUI

struct ContentView: View {
    @StateObject private var client = DaemonClient()
    @State private var selectedWorkspace: Workspace?
    @State private var selectedTerminal: Terminal?
    @State private var output = ""
    @State private var input = ""
    @State private var pollTask: Task<Void, Never>?

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            detail
        }
        .task {
            await client.refresh()
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
                output: $output,
                input: $input,
                onSend: {
                    let text = input
                    input = ""
                    Task {
                        await client.send(terminal: terminal.short, text: text + "\n")
                        try? await Task.sleep(for: .milliseconds(350))
                        output = await client.capture(terminal: terminal.short)
                    }
                },
                onRestart: { Task { await client.restart(terminal: terminal.short) } },
                onDismiss: { Task { await client.dismissLost(terminal: terminal.short) } },
                onStop: { Task { await client.stop(terminal: terminal.short) } }
            )
            .task(id: terminal.id) {
                output = await client.capture(terminal: terminal.short)
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

    private func startPolling() {
        pollTask?.cancel()
        pollTask = Task {
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(3))
                if Task.isCancelled { return }
                await client.refresh()
                if let t = selectedTerminal {
                    output = await client.capture(terminal: t.short)
                }
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
    @Binding var output: String
    @Binding var input: String
    let onSend: () -> Void
    let onRestart: () -> Void
    let onDismiss: () -> Void
    let onStop: () -> Void

    private var kind: StateKind { StateKind.parse(terminal.state) }

    var body: some View {
        VStack(spacing: 0) {
            header

            ScrollViewReader { proxy in
                ScrollView {
                    Text(output.isEmpty ? "(no output yet)" : output)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(12)
                        .id("bottom")
                }
                .background(Color(nsColor: .textBackgroundColor))
                .onChange(of: output) { _, _ in
                    proxy.scrollTo("bottom", anchor: .bottom)
                }
            }

            composer
        }
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
    private var composer: some View {
        Divider()
        if kind == .running {
            HStack(spacing: 8) {
                TextField("Send to terminal", text: $input)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(.body, design: .monospaced))
                    .onSubmit(onSend)
                Button("Send", action: onSend).disabled(input.isEmpty)
            }
            .padding(12)
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
