import SwiftUI

// One workspace, pushed from Needs You (spec §6.1): its name as the title,
// and a segmented Orchestrator, Board and Worktrees control under it,
// remembered per workspace (`WorkspaceSegment`).
//
// - Orchestrator: its pane, full height, in the existing terminal and agent
//   views. With none, Start Orchestrator (ruling 8); lost, Restart and
//   Replace; starting, "Starting Orchestrator…" and, after 30 seconds, the
//   warning that the seat can stick (spec §8).
// - Board: the list form, in-line (`WorkspaceBoardList`). A card pushes its
//   task, so the board is never covered by a jump.
// - Worktrees: the ones it owns, with their tasks and changes. New Worktree…
//   claims the worktree for this workspace.

struct WorkspaceScreen: View {
    @ObservedObject var fleet: FleetStore
    @ObservedObject var hosts: RunnerStore
    @ObservedObject var connection: Connection
    let place: PhoneWorkspace

    /// The segment on screen, kept per workspace (`WorkspaceSegment`).
    @State private var segment: WorkspaceSegment

    init(fleet: FleetStore, hosts: RunnerStore, connection: Connection, place: PhoneWorkspace) {
        self.fleet = fleet
        self.hosts = hosts
        self.connection = connection
        self.place = place
        _segment = State(
            initialValue: WorkspaceSegment.remembered(
                place, implicit: connection.workspace(place.workspace)?.isImplicit ?? false))
    }

    @Environment(\.phoneNavigator) private var navigator

    var body: some View {
        Group {
            if let summary {
                content(summary)
            } else {
                ContentUnavailableView {
                    Label("Workspace Gone", systemImage: "square.stack.3d.up.slash")
                } description: {
                    Text("This workspace isn’t on its runner anymore.")
                }
            }
        }
        .navigationTitle(summary.map(title) ?? "Workspace")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear {
            UserDefaults.standard.set(place.stored, forKey: PhoneLaunch.lastWorkspaceKey)
        }
    }

    private var summary: WorkspaceSummary? { connection.workspace(place.workspace) }

    /// An implicit workspace is called by its repository, as its row was.
    private func title(_ summary: WorkspaceSummary) -> String {
        guard summary.isImplicit else { return summary.name }
        return connection.repositoryNames[summary.id] ?? summary.name
    }

    /// The segment to draw: the one chosen, unless this workspace doesn't
    /// offer it.
    private func current(_ summary: WorkspaceSummary) -> WorkspaceSegment {
        let offered = WorkspaceSegment.offered(implicit: summary.isImplicit)
        return offered.contains(segment) ? segment : offered[0]
    }

    @ViewBuilder
    private func content(_ summary: WorkspaceSummary) -> some View {
        let shown = current(summary)
        Group {
            switch shown {
            case .orchestrator:
                OrchestratorSegment(connection: connection, summary: summary)
            case .board:
                WorkspaceBoardList(
                    board: connection.boards[summary.id],
                    unread: connection.unreadBoards.contains(summary.id),
                    place: place,
                    speaksOfAgents: TaskAgentLink.speaksOfAgents(
                        connected: connection.isAnswering, build: connection.daemon),
                    agents: connection.boardAgents(for:),
                    onOpen: { row in navigator?.open(.task(place, task: row.id)) },
                    onJump: { agent in openAgent(agent) },
                    onRefresh: { await connection.readBoard(summary) })
                .task { await connection.readBoard(summary) }
            case .worktrees:
                WorkspaceWorktrees(connection: connection, summary: summary, place: place)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // A bar of the navigation bar's own, so the control sits in its
        // material and takes its touches, rather than under its edge.
        .safeAreaBar(edge: .top) {
            SegmentBar(
                segments: WorkspaceSegment.offered(implicit: summary.isImplicit),
                selection: $segment)
        }
        .onChange(of: segment) { _, chosen in chosen.remember(for: place) }
    }

    /// A card's Agent button: the agent's worktree, on its pane, pushed over
    /// the board.
    private func openAgent(_ agent: BoardAgent) {
        guard
            let worktree = connection.fleet.worktrees.first(where: {
                $0.terminals.contains { $0.id == agent.id }
            })
        else { return }
        navigator?.open(.worktree(runner: place.runner, worktree: worktree.id, landing: .terminal(agent.id)))
    }
}

/// Orchestrator, Board and Worktrees, one of them chosen.
///
/// Buttons in a capsule rather than a segmented `Picker`: in this spot the
/// system control dropped most of the taps the UI suite sent it (seen with
/// `-phone-harness`; the cause wasn't found), so a test could not choose a
/// segment reliably. Plain buttons take every tap.
private struct SegmentBar: View {
    let segments: [WorkspaceSegment]
    @Binding var selection: WorkspaceSegment

    var body: some View {
        HStack(spacing: 2) {
            ForEach(segments, id: \.self) { segment in
                let chosen = segment == selection
                Button {
                    selection = segment
                } label: {
                    Text(segment.title)
                        .font(.subheadline.weight(chosen ? .semibold : .regular))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 6)
                        .background {
                            if chosen {
                                Capsule().fill(Color.primary.opacity(0.16))
                            }
                        }
                        .contentShape(.capsule)
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(chosen ? .isSelected : [])
                .accessibilityIdentifier("segment-\(segment.rawValue)")
            }
        }
        .padding(3)
        .background(Capsule().fill(Color.primary.opacity(0.07)))
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("workspace-segments")
    }
}

// MARK: - Orchestrator

/// The workspace's orchestrator, or what to do about not having one.
private struct OrchestratorSegment: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary

    @StateObject private var pastes = ImagePasteQueue()
    /// When this phone asked for one, until its pane appears.
    @State private var startedAt: Date?
    /// What the runner said when it refused, in this app's words.
    @State private var refusal: String?
    @State private var replacing = false

    /// The harnesses an orchestrator can run on.
    static let harnesses: [(id: String, name: String)] = [
        ("claude", "Claude"), ("codex", "Codex"), ("cursor", "Cursor"),
    ]

    var body: some View {
        VStack(spacing: 0) {
            if let refusal {
                Label(refusal, systemImage: "exclamationmark.triangle.fill")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 8)
                    .accessibilityIdentifier("orchestrator-refusal")
            }
            state
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .confirmationDialog(
            "Replace the Orchestrator?", isPresented: $replacing, titleVisibility: .visible
        ) {
            ForEach(Self.harnesses, id: \.id) { harness in
                Button("Replace with \(harness.name)", role: .destructive) {
                    start(harness.id, replace: true)
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This stops the orchestrator that’s running and starts a new one.")
        }
        .onChange(of: orchestrator?.id) { _, id in
            if id != nil { startedAt = nil }
        }
    }

    /// The orchestrator's pane, as the fleet has it: the workspace's own
    /// word, else the pane that says it leads this workspace.
    private var orchestrator: Terminal? {
        let all = connection.fleet.worktrees.lazy.flatMap(\.terminals)
        if let id = summary.orchestrator, let found = all.first(where: { $0.id == id }) {
            return found
        }
        return all.first { $0.isOrchestrator && $0.workspace == summary.id }
    }

    @ViewBuilder
    private var state: some View {
        if let terminal = orchestrator {
            switch StateKind.parse(terminal.state) {
            case .lost, .exited, .error:
                lost(terminal)
            case .starting:
                starting(since: startedAt ?? Date())
            default:
                OrchestratorPane(terminal: terminal, connection: connection, pastes: pastes)
                    .id(terminal.id)
            }
        } else if let startedAt {
            starting(since: startedAt)
        } else {
            none
        }
    }

    private var none: some View {
        ContentUnavailableView {
            Label("No Orchestrator", systemImage: "circle.dashed")
        } description: {
            Text(
                "An orchestrator runs this workspace’s board. It reads the charter, dispatches "
                    + "agents, and asks you when it needs a decision.")
        } actions: {
            if mayAct {
                Menu("Start Orchestrator") {
                    ForEach(Self.harnesses, id: \.id) { harness in
                        Button(harness.name) { start(harness.id, replace: false) }
                    }
                }
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier("start-orchestrator")
            }
        }
    }

    private func starting(since: Date) -> some View {
        TimelineView(.periodic(from: since, by: 1)) { context in
            VStack(spacing: 12) {
                ProgressView()
                Text("Starting Orchestrator…")
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("orchestrator-starting")
                if context.date.timeIntervalSince(since) >= Self.slowStart {
                    Text("This is taking longer than usual.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    if mayAct {
                        Button("Replace…") { replacing = true }
                            .buttonStyle(.bordered)
                    }
                }
            }
        }
    }

    /// Whether this phone may start, restart or replace an orchestrator:
    /// not on a Read grant (as Android's `mayControl`).
    private var mayAct: Bool { connection.daemon?.mayAct ?? true }

    /// How long a start may take before the screen says so. The seat can
    /// stick (spec §8), and Replace is the way past it.
    private static let slowStart: TimeInterval = 30

    private func lost(_ terminal: Terminal) -> some View {
        ContentUnavailableView {
            Label("Orchestrator Stopped", systemImage: "exclamationmark.triangle")
        } description: {
            Text("It isn’t running. Restart picks the conversation up where it left off.")
        } actions: {
            if mayAct {
                Menu("Orchestrator") {
                    Button("Restart") {
                        Task { await connection.act(.restart, on: terminal) }
                    }
                    Button("Replace…") { replacing = true }
                }
                .buttonStyle(.bordered)
                .accessibilityIdentifier("orchestrator-lost")
            }
        }
    }

    private func start(_ harness: String, replace: Bool) {
        refusal = nil
        startedAt = Date()
        Task {
            do {
                _ = try await connection.startOrchestrator(
                    workspace: summary.id, harness: harness, replace: replace)
            } catch {
                startedAt = nil
                refusal = ClientCore.trouble(error, after: "The runner didn’t start the orchestrator.")
                    .sentence
            }
        }
    }
}

/// The orchestrator's own pane, full height: the same terminal and agent
/// views a worktree's panes use, and the one pane being read while it's up.
private struct OrchestratorPane: View {
    let terminal: Terminal
    @ObservedObject var connection: Connection
    @ObservedObject var pastes: ImagePasteQueue

    var body: some View {
        TerminalView(terminal: terminal, isVisible: true, connection: connection, pastes: pastes)
            .background(TerminalPalette.background.ignoresSafeArea(edges: .bottom))
            .accessibilityIdentifier("orchestrator-pane")
            .onAppear {
                Notifier.shared.visibleTerminal = terminal.id
                Task { await connection.markVisibleSeen() }
            }
            .onDisappear {
                if Notifier.shared.visibleTerminal == terminal.id {
                    Notifier.shared.visibleTerminal = nil
                }
            }
    }
}

// MARK: - Worktrees

/// The workspace's worktrees, and the way to make another.
private struct WorkspaceWorktrees: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let place: PhoneWorkspace

    @State private var composing = false
    @State private var fromBranch = false

    @Environment(\.phoneNavigator) private var navigator

    var body: some View {
        List {
            if owned.isEmpty {
                Section {
                    Text("No worktrees yet. The orchestrator makes them as it dispatches tasks.")
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("worktrees-empty")
                }
            } else {
                Section {
                    ForEach(owned) { worktree in row(worktree) }
                }
            }
            if !hidden.isEmpty {
                Section("Hidden") {
                    ForEach(hidden) { worktree in row(worktree) }
                }
            }
            Section {
                Button("New Worktree…") { composing = true }
                    .accessibilityIdentifier("new-worktree")
                Button("From a Branch…") { fromBranch = true }
            }
        }
        .listStyle(.insetGrouped)
        .sheet(isPresented: $composing) {
            TaskComposerView(connection: connection, workspace: summary)
        }
        .sheet(isPresented: $fromBranch) {
            NewWorktreeView(
                repositories: connection.repositories.filter {
                    summary.repository == nil || $0.id == summary.repository
                },
                connection: connection
            ) { repository, name, branch, adopt in
                await connection.createWorktree(
                    repository: repository, name: name, branch: branch, adopt: adopt,
                    workspace: summary.boardWorkspace)
            }
        }
    }

    private func row(_ worktree: Worktree) -> some View {
        WorktreeRow(worktree: worktree, inbox: connection.inbox[worktree.id]) {
            navigator?.open(.worktree(runner: place.runner, worktree: worktree.id, landing: .resume))
        }
    }

    /// Every worktree this workspace owns, or on a runner without
    /// workspaces, every one in its repository.
    private var all: [Worktree] {
        connection.fleet.worktrees.filter { worktree in
            summary.isImplicit
                ? worktree.repository == summary.id : worktree.workspace == summary.id
        }
    }

    private var owned: [Worktree] { all.filter { !$0.isHidden } }
    private var hidden: [Worktree] { all.filter(\.isHidden) }
}
