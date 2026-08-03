import SwiftUI

/// The fleet on one host.
///
/// Was the screen between a host and a terminal; now it is mostly a
/// hand-off. A host with a terminal already running goes straight to it
/// (`landing`, decided once per connection — see below) and this screen is
/// what a host with nothing running shows instead, and what `TerminalView`'s
/// switcher sheet reuses to list every worktree. Every state shown here is
/// still DERIVED by the daemon at the moment of asking — the phone never
/// computes a terminal's state, because a client that re-derives can
/// disagree with the daemon and with the Mac about the same terminal.
@MainActor
struct FleetView: View {
    let host: Host
    let store: HostStore

    @StateObject private var connection = Connection()

    /// Which terminal to open automatically, decided once per connection
    /// attempt and then left alone.
    ///
    /// Recomputing this on every poll would mean a terminal finishing its
    /// work while this screen is open — an ordinary thing to happen while
    /// someone is reading it — yanks them onto a different pane mid-read.
    /// `nil` with `landingDecided` true means the fleet genuinely has no
    /// terminals, which is what falls back to `list` below; `nil` with it
    /// false means a connection attempt hasn't finished yet.
    @State private var landing: Terminal?
    @State private var landingDecided = false

    /// Pushed when a terminal is tapped in the fallback list — the only
    /// place this screen still pushes, since the landing terminal above
    /// takes the direct route and the switcher sheet (`WorkspaceListView`
    /// from `TerminalView`) selects in place instead of navigating.
    @State private var pushed: Terminal?

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
                .navigationTitle(host.label)
                .navigationBarTitleDisplayMode(.inline)

            case .needsApproval(let fingerprint):
                approval(fingerprint)
                    .navigationTitle(host.label)
                    .navigationBarTitleDisplayMode(.inline)

            case .failed(let message):
                failure(message)
                    .navigationTitle(host.label)
                    .navigationBarTitleDisplayMode(.inline)

            case .connected:
                connected
            }
        }
        .navigationDestination(item: $pushed) { terminal in
            TerminalView(terminal: terminal, connection: connection, hosts: store)
        }
        .task { await connect(host) }
    }

    @ViewBuilder
    private var connected: some View {
        if let landing {
            TerminalView(terminal: landing, connection: connection, hosts: store)
        } else if landingDecided {
            WorkspaceListView(connection: connection, onSelect: { pushed = $0 }, hosts: store)
                .navigationTitle(host.label)
                .navigationBarTitleDisplayMode(.inline)
        } else {
            ProgressView()
                .navigationTitle(host.label)
                .navigationBarTitleDisplayMode(.inline)
        }
    }

    /// Connect, then decide `landing` from whatever fleet that connection
    /// produced. Shared by the initial `.task` and every retry below — the
    /// approval screen's "Trust this host" and the failure screen's "Try
    /// again" each start a fresh connection of their own, and each one needs
    /// the same decision made afterwards, not the one left over from a
    /// connection attempt that never got this far.
    private func connect(_ target: Host) async {
        await connection.start(host: target)
        landing = connection.fleet.landingTerminal
        landingDecided = true
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
                var trusted = host
                trusted.fingerprint = fingerprint
                Task { await connect(trusted) }
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
            Button("Try again") { Task { await connect(host) } }
                .buttonStyle(.bordered)
        }
        .padding()
    }
}

/// The worktree list plus what it takes to act on it: quick task, new
/// workspace, pull-to-refresh. Shown two places — as `FleetView`'s own
/// fallback when a host has no terminal to land on, and inside the sheet
/// `TerminalView` opens to switch terminals — so a task started from either
/// one works the same way and neither loses a capability the other has.
struct WorkspaceListView: View {
    @ObservedObject var connection: Connection
    let onSelect: (Terminal) -> Void
    /// Non-nil only in the sheet: what "Done" calls. `FleetView`'s own use
    /// leaves this nil because a pushed screen already has a back button.
    var onDismiss: (() -> Void)?
    /// The machines to switch between, when this is the sheet.
    ///
    /// Switching hosts lives here because this is already where you go to
    /// switch what you are looking at. The app opens onto terminals now (see
    /// `RootView`), so there is no host list to go back to — and inventing a
    /// second switcher screen for the rarer of the two switches would be one
    /// more place to look.
    var hosts: HostStore?

    @State private var showNewWorkspace = false
    @State private var showQuickTask = false
    @State private var showAddHost = false
    @State private var showSettings = false

    var body: some View {
        FleetList(fleet: connection.fleet, onSelect: onSelect) { action, terminal in
            Task { await connection.act(action, on: terminal) }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if let hosts, hosts.hosts.count > 1 || hosts.selected != nil {
                hostSwitcher(hosts)
            }
        }
        .refreshable { await connection.refresh() }
        .toolbar {
            // Sparkles for "describe it" (QuickTaskView), plain plus for
            // "fill in the form" (NewWorkspaceView) — same two flows the
            // Mac keeps side by side, kept apart here by icon rather than
            // by picking a winner, since a phone's one-sentence flow is
            // new and unproven next to a form that already works.
            ToolbarItem(placement: .topBarTrailing) {
                Button { showQuickTask = true } label: { Image(systemName: "sparkles") }
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button { showNewWorkspace = true } label: { Image(systemName: "plus") }
            }
            if let onDismiss {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: onDismiss)
                }
            }
        }
        .sheet(isPresented: $showNewWorkspace) {
            NewWorkspaceView(repositories: connection.repositories) { repository, task, branch in
                await connection.createWorkspace(repository: repository, task: task, branch: branch)
            }
        }
        .sheet(isPresented: $showQuickTask) {
            TaskComposerView(connection: connection)
        }
        .sheet(isPresented: $showAddHost) {
            AddHostView { host in hosts?.add(host) }
        }
        .sheet(isPresented: $showSettings) {
            NavigationStack {
                SettingsView(connection: connection)
                    .toolbar {
                        ToolbarItem(placement: .topBarLeading) {
                            NavigationLink("Authorise") { AuthoriseView() }
                        }
                    }
            }
        }
    }

    /// Which machine these terminals are on, and how to change it.
    ///
    /// A strip along the bottom rather than a section in the list: the list is
    /// worktrees on ONE host, and putting the host inside it would read as one
    /// more thing in the same collection. This says what the collection belongs
    /// to.
    private func hostSwitcher(_ hosts: HostStore) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "server.rack")
                .font(.caption)
                .foregroundStyle(.secondary)

            Menu {
                ForEach(hosts.hosts) { host in
                    Button {
                        hosts.selected = host
                        onDismiss?()
                    } label: {
                        if host.id == hosts.selected?.id {
                            Label(host.label, systemImage: "checkmark")
                        } else {
                            Text(host.label)
                        }
                    }
                }
                Divider()
                Button("Add a host…") { showAddHost = true }
                // Reachable from here because there is nowhere else left.
                //
                // Settings and the device's public key used to live on the root
                // screen, which was the host list. The app opens onto terminals
                // now, so that screen only appears when there are no hosts —
                // and everything that was on it would have become unreachable
                // the moment you added one.
                Button("This device…") { showSettings = true }
            } label: {
                HStack(spacing: 4) {
                    Text(hosts.selected?.label ?? "No host")
                        .font(.callout.weight(.medium))
                    Image(systemName: "chevron.up.chevron.down").font(.caption2)
                }
            }

            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar)
    }
}

/// The rows themselves: every workspace on a host, its terminals, and the
/// swipe actions on each. Split out of `WorkspaceListView` because the two
/// places that view is shown agree on everything except the frame around the
/// list — a `NavigationStack` with a title and a "Done" button in the sheet,
/// nothing of the kind when it's `FleetView`'s own fallback content — and
/// `List` content itself carries no opinion about what encloses it.
struct FleetList: View {
    let fleet: Fleet
    let onSelect: (Terminal) -> Void
    let onAction: (Connection.Action, Terminal) -> Void

    var body: some View {
        List {
            if fleet.workspaces.isEmpty {
                Text("No workspaces on this host.")
                    .foregroundStyle(.secondary)
            }

            ForEach(fleet.workspaces) { workspace in
                let numbering = workspace.ordinals()
                Section {
                    // Anything waiting on you comes first. On a phone you see
                    // four rows at a time, and scrolling to find the one that
                    // needs an answer defeats the purpose of the screen.
                    ForEach(workspace.terminals.sorted { a, b in
                        a.agent.wantsAttention && !b.agent.wantsAttention
                    }) { terminal in
                        Button { onSelect(terminal) } label: {
                            TerminalRow(terminal: terminal, ordinal: numbering[terminal.id]) { action in
                                onAction(action, terminal)
                            }
                        }
                        .buttonStyle(.plain)
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
                        .fill(fleet.runtimeHealthy ? .green : .orange)
                        .frame(width: 8, height: 8)
                    Text(
                        fleet.runtimeHealthy
                            ? "\(fleet.livePanes) live"
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
    /// Which of several identically-labelled siblings this is, from
    /// `Workspace.ordinals()`, or nil when its label is unique in the
    /// workspace and numbering it would answer a question nobody asked.
    var ordinal: Int?
    let onAction: (Connection.Action) -> Void

    private var kind: StateKind { StateKind.parse(terminal.state) }

    var body: some View {
        HStack(spacing: 10) {
            Circle().fill(processColour(kind)).frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    Text(terminal.label).font(.body)
                    if let ordinal {
                        Text("\(ordinal)")
                            .font(.caption2.monospaced())
                            .foregroundStyle(.tertiary)
                    }
                }
                Text(terminal.state.lowercased())
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
                    .foregroundStyle(attentionColour(terminal.agent))
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
}

/// The dot colour for "is the process alive" — shared by the fleet list and
/// the terminal tab strip (`TerminalTabStrip`), so the same terminal cannot
/// read green in one screen and red in the other.
func processColour(_ kind: StateKind) -> Color {
    switch kind {
    case .running: return .green
    case .starting: return .yellow
    case .exited: return .secondary
    // The one state that means Far Cooler does not know what happened.
    case .lost, .error: return .red
    case .unknown: return .secondary
    }
}

/// The colour behind an agent's activity glyph, shared with the tab strip for
/// the same reason as `processColour` above.
func attentionColour(_ agent: AgentActivity) -> Color {
    switch agent {
    case .blocked: return .orange
    case .done: return .green
    default: return .secondary
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
