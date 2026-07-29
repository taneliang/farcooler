import SwiftUI

/// The fleet on one host.
///
/// This is the 3am screen: what is running, what needs attention, and what to
/// do about it. Every state shown here is DERIVED by the daemon at the moment
/// of asking — the phone never computes a terminal's state, because a client
/// that re-derives can disagree with the daemon and with the Mac about the same
/// terminal.
@MainActor
struct FleetView: View {
    let host: Host
    let store: HostStore

    @StateObject private var connection = Connection()
    @State private var showNewWorkspace = false

    var body: some View {
        Group {
            switch connection.phase {
            case .connecting:
                VStack(spacing: 12) {
                    ProgressView()
                    Text("Connecting to \(host.address)…")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }

            case .needsApproval(let fingerprint):
                approval(fingerprint)

            case .failed(let message):
                failure(message)

            case .connected:
                fleet
            }
        }
        .navigationTitle(host.label)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if connection.phase == .connected {
                Button { showNewWorkspace = true } label: { Image(systemName: "plus") }
            }
        }
        .sheet(isPresented: $showNewWorkspace) {
            NewWorkspaceView(repositories: connection.repositories) { repository, task, branch in
                await connection.createWorkspace(repository: repository, task: task, branch: branch)
            }
        }
        .task { await connection.start(host: host) }
        .refreshable { await connection.refresh() }
    }

    // MARK: - Phases

    /// First contact. The fingerprint is shown and refused until a human says
    /// yes, because silently trusting an unknown key is what makes an
    /// interception invisible.
    private func approval(_ fingerprint: String) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Unrecognised host", systemImage: "questionmark.circle")
                .font(.headline)
            Text("\(host.address) presented this key:")
                .font(.callout)
            Text(fingerprint)
                .font(.system(.footnote, design: .monospaced))
                .textSelection(.enabled)
                .padding(12)
                .background(Color(.secondarySystemBackground))
                .clipShape(RoundedRectangle(cornerRadius: 10))
            Text(
                "Check it against the host: ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub"
            )
            .font(.footnote)
            .foregroundStyle(.secondary)

            Button("Trust this host") {
                store.trust(host, fingerprint: fingerprint)
                Task {
                    var trusted = host
                    trusted.fingerprint = fingerprint
                    await connection.start(host: trusted)
                }
            }
            .buttonStyle(.borderedProminent)
            Spacer()
        }
        .padding()
    }

    private func failure(_ message: String) -> some View {
        VStack(spacing: 14) {
            Image(systemName: "exclamationmark.triangle")
                .font(.largeTitle)
                .foregroundStyle(.orange)
            Text("Could not connect").font(.headline)
            Text(message)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .textSelection(.enabled)
            Button("Try again") { Task { await connection.start(host: host) } }
                .buttonStyle(.bordered)
        }
        .padding()
    }

    private var fleet: some View {
        List {
            if connection.fleet.workspaces.isEmpty {
                Text("No workspaces on this host.")
                    .foregroundStyle(.secondary)
            }

            ForEach(connection.fleet.workspaces) { workspace in
                Section {
                    // Anything waiting on you comes first. On a phone you see
                    // four rows at a time, and scrolling to find the one that
                    // needs an answer defeats the purpose of the screen.
                    ForEach(workspace.terminals.sorted { a, b in
                        a.agent.wantsAttention && !b.agent.wantsAttention
                    }) { terminal in
                        TerminalRow(terminal: terminal) { action in
                            Task { await connection.act(action, on: terminal) }
                        }
                    }
                    if workspace.terminals.isEmpty {
                        Text("No terminals").font(.callout).foregroundStyle(.secondary)
                    }
                } header: {
                    HStack {
                        Text(workspace.task)
                        Spacer()
                        Text(workspace.branch)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                    }
                }
            }

            Section {
                HStack {
                    Circle()
                        .fill(connection.fleet.runtimeHealthy ? .green : .orange)
                        .frame(width: 8, height: 8)
                    Text(
                        connection.fleet.runtimeHealthy
                            ? "\(connection.fleet.livePanes) live"
                            : "tmux unavailable on this host"
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
        }
    }
}

struct TerminalRow: View {
    let terminal: Terminal
    let onAction: (Connection.Action) -> Void

    private var kind: StateKind { StateKind.parse(terminal.state) }

    var body: some View {
        HStack(spacing: 10) {
            Circle().fill(colour).frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 1) {
                Text(terminal.title).font(.body)
                Text("\(terminal.preset) · \(terminal.state.lowercased())")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()

            // The reason to have opened the app. Only the two states worth
            // acting on get colour, so a list of twenty still reads at a glance.
            if terminal.agent.isAgent && terminal.agent != .unknown {
                Label(terminal.agent.label, systemImage: terminal.agent.symbol)
                    .labelStyle(.iconOnly)
                    .font(.system(size: 13, weight: terminal.agent.wantsAttention ? .semibold : .regular))
                    .foregroundStyle(activityColour)
                    .accessibilityLabel(terminal.agent.label)
            }
        }
        .swipeActions(edge: .trailing) {
            if kind == .lost {
                Button("Dismiss") { onAction(.dismissLost) }.tint(.gray)
            }
            Button("Restart") { onAction(.restart) }.tint(.blue)
            if kind == .running || kind == .starting {
                Button("Stop", role: .destructive) { onAction(.stop) }
            }
        }
    }

    private var activityColour: Color {
        switch terminal.agent {
        case .blocked: return .orange
        case .done: return .green
        default: return .secondary
        }
    }

    private var colour: Color {
        switch kind {
        case .running: return .green
        case .starting: return .yellow
        case .exited: return .secondary
        // The one state that means Overnight does not know what happened.
        case .lost, .error: return .red
        case .unknown: return .secondary
        }
    }
}

struct NewWorkspaceView: View {
    let repositories: [Repository]
    let onCreate: (String, String, String) async -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var repository: String = ""
    @State private var task = ""
    @State private var branch = ""
    @State private var working = false

    private var isValid: Bool {
        !repository.isEmpty && !task.trimmingCharacters(in: .whitespaces).isEmpty
            && !branch.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        NavigationStack {
            Form {
                Picker("Repository", selection: $repository) {
                    Text("Choose").tag("")
                    ForEach(repositories) { Text($0.displayName).tag($0.id) }
                }
                TextField("Task", text: $task)
                TextField("Branch", text: $branch)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Section {
                    Text("A workspace is one git worktree and branch, for one task.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
            }
            .navigationTitle("New workspace")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Create") {
                        working = true
                        Task {
                            await onCreate(repository, task, branch)
                            working = false
                            dismiss()
                        }
                    }
                    .disabled(!isValid || working)
                }
            }
        }
    }
}
