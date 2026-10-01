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
//   task, so the board is never covered by a jump. New Task… files one
//   (`NewTaskSheet`), below a Read grant.
// - Worktrees: the ones it owns, with their tasks and changes. New Worktree…
//   claims the worktree for this workspace.

struct WorkspaceScreen: View {
    @ObservedObject var fleet: FleetStore
    @ObservedObject var hosts: RunnerStore
    @ObservedObject var connection: Connection
    let place: PhoneWorkspace

    /// The segment on screen, kept per workspace (`WorkspaceSegment`).
    @State private var segment: WorkspaceSegment
    @State private var filing = false

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
        ZStack {
            // Mounted whenever the workspace has the segment, whichever is
            // up, and live only on its own (ov-66, the owner's ruling 2), as
            // Android keeps it: going to the board and back neither
            // reconnects the pane nor loses its place. Hidden, it holds no
            // stream (`TerminalView`'s `isVisible`) and takes no touches.
            if WorkspaceSegment.offered(implicit: summary.isImplicit).contains(.orchestrator) {
                let up = shown == .orchestrator
                OrchestratorSegment(connection: connection, summary: summary, place: place, shown: up)
                    .opacity(up ? 1 : 0)
                    .allowsHitTesting(up)
                    .accessibilityHidden(!up)
            }
            switch shown {
            case .orchestrator:
                EmptyView()
            case .board:
                WorkspaceBoardList(
                    board: connection.boards[summary.id],
                    unread: connection.unreadBoards.contains(summary.id),
                    place: place,
                    speaksOfAgents: TaskAgentLink.speaksOfAgents(
                        connected: connection.isAnswering, build: connection.daemon),
                    waiting: RunnerBoards.waiting(
                        on: connection.boards[summary.id], in: summary, items: connection.needsYou,
                        listRead: connection.needsYouRead && !connection.needsYouDerived,
                        build: connection.daemon),
                    agents: connection.boardAgents(for:),
                    onOpen: { row in navigator?.open(.task(place, task: row.id)) },
                    onJump: { agent in openAgent(agent) },
                    onRefresh: { await connection.readBoard(summary) },
                    onNewTask: PhoneNewTask.offered(connection.daemon) ? { filing = true } : nil)
                .task { await connection.readBoard(summary) }
            case .worktrees:
                WorkspaceWorktrees(connection: connection, summary: summary, place: place)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .toolbar {
            if shown == .board && PhoneNewTask.offered(connection.daemon) {
                ToolbarItem(placement: .primaryAction) {
                    Button("New Task…", systemImage: "plus") { filing = true }
                        .accessibilityIdentifier("new-task")
                }
            }
        }
        .sheet(isPresented: $filing) {
            NewTaskSheet(connection: connection, workspace: summary)
        }
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

    /// Side by side when the three titles fit on one line each, stacked when
    /// they don't. A title is never allowed to wrap: at the larger Dynamic Type
    /// sizes "Orchestrator" broke as "Orches-/trator" and "Worktrees" as
    /// "Work-/trees", which is a control nobody can read at a glance.
    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 2) { buttons(stacked: false) }
            VStack(spacing: 2) { buttons(stacked: true) }
        }
        .padding(3)
        .background(Capsule().fill(Color.primary.opacity(0.07)))
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("workspace-segments")
    }

    @ViewBuilder
    private func buttons(stacked: Bool) -> some View {
        ForEach(segments, id: \.self) { segment in
            let chosen = segment == selection
            Button {
                selection = segment
            } label: {
                Text(segment.title)
                    .font(.subheadline.weight(chosen ? .semibold : .regular))
                    .lineLimit(1)
                    // One line, always: the unstacked bar must report its
                    // true width for `ViewThatFits` to see that it doesn't fit.
                    .fixedSize(horizontal: !stacked, vertical: true)
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
}

// MARK: - Orchestrator

/// The workspace's orchestrator, or what to do about not having one.
private struct OrchestratorSegment: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    /// Whether it's the segment up, rather than mounted under another.
    let shown: Bool

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
                OrchestratorPane(
                    terminal: terminal, connection: connection, pastes: pastes, place: place,
                    shown: shown)
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
            Label("No Orchestrator", systemImage: "person.crop.circle.badge.plus")
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
///
/// **Live only while it's up and nothing covers it** (ov-66 fix): its
/// segment chosen, and its workspace the top of the stack, with no task
/// pushed over it and no worktree covering it. Covered, it stays mounted,
/// as ruled, and its session stops (`TerminalView`'s `isVisible`): a
/// stream nobody can see is a phone's battery and data spent for nothing.
///
/// Being read is `TerminalView`'s to claim, as every pane's is: while
/// `isVisible`, it tells `Notifier` and marks the pane seen, and it claims
/// again when it comes back, which is what Back from a cover does. What
/// that claim never does is give itself back when the pane is only hidden,
/// so this does, whenever it stops being live, and when the screen goes.
private struct OrchestratorPane: View {
    let terminal: Terminal
    @ObservedObject var connection: Connection
    @ObservedObject var pastes: ImagePasteQueue
    let place: PhoneWorkspace
    let shown: Bool

    @Environment(\.phoneNavigator) private var navigator

    var body: some View {
        if let navigator {
            CoverReader(navigator: navigator, place: place) { covered in
                pane(live: shown && !covered)
            }
        } else {
            pane(live: shown)
        }
    }

    private func pane(live: Bool) -> some View {
        OrchestratorTerminal(
            terminal: terminal, connection: connection, pastes: pastes, live: live)
    }
}

/// Whether anything is over `place`'s screen on the phone's stack: a
/// screen pushed over it, or a worktree covering the stack.
///
/// A view of its own so that only what it wraps watches the stack: the
/// workspace screen and its segment control never re-render on a push
/// (see `PhoneNavigator`).
private struct CoverReader<Content: View>: View {
    @ObservedObject var navigator: PhoneNavigator
    let place: PhoneWorkspace
    @ViewBuilder let content: (Bool) -> Content

    var body: some View {
        content(navigator.worktree != nil || navigator.path.last != .workspace(place))
    }
}

/// The orchestrator's terminal, live or not.
private struct OrchestratorTerminal: View {
    let terminal: Terminal
    @ObservedObject var connection: Connection
    @ObservedObject var pastes: ImagePasteQueue
    let live: Bool

    #if DEBUG
    /// Which mount of the pane this is, for a UI test to tell a pane kept
    /// mounted from one built again (`orchestrator-mount`).
    @StateObject private var mount = PaneMount()
    #endif

    var body: some View {
        TerminalView(terminal: terminal, isVisible: live, connection: connection, pastes: pastes)
            .background(TerminalPalette.background.ignoresSafeArea(edges: .bottom))
            // A terminal surface in a screen whose chrome is the system's: its
            // own text (the status states) reads against the theme's ground,
            // so the scheme is the theme's from here down and no further.
            .environment(\.colorScheme, Themes.shared.current.colorScheme)
            .accessibilityIdentifier("orchestrator-pane")
            .onChange(of: live) { _, now in
                if !now { Notifier.shared.release(terminal.id) }
            }
            .onDisappear { Notifier.shared.release(terminal.id) }
            #if DEBUG
            .onAppear { PhoneProbe.shared.orchestrators.insert(terminal.id) }
            #endif
            #if DEBUG
            .overlay(alignment: .topTrailing) {
                Rectangle()
                    .fill(Color.white.opacity(0.001))
                    .frame(width: 1, height: 1)
                    .accessibilityElement()
                    .accessibilityIdentifier("orchestrator-mount")
                    .accessibilityValue("mount=\(mount.serial)")
            }
            #endif
    }
}

#if DEBUG
/// A count of the orchestrator panes built, one per mount: a `StateObject`
/// is made once for as long as its view is in the tree.
@MainActor
private final class PaneMount: ObservableObject {
    private static var built = 0
    let serial: Int

    init() {
        Self.built += 1
        serial = Self.built
    }
}
#endif

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

    /// A row, with the Mac's Hide and Unhide on its swipe: a worktree put away
    /// stays in its workspace's Hidden section and this is the way back out.
    /// Absent below a Control grant, like every other write on this screen.
    private func row(_ worktree: Worktree) -> some View {
        WorktreeRow(worktree: worktree, inbox: connection.inbox[worktree.id]) {
            navigator?.open(.worktree(runner: place.runner, worktree: worktree.id, landing: .resume))
        }
        .swipeActions(edge: .trailing) {
            if connection.daemon?.mayAct ?? true {
                if worktree.isHidden {
                    Button("Unhide") { Task { await connection.unhideWorktree(worktree) } }
                        .tint(.blue)
                        .accessibilityIdentifier("unhide-\(worktree.task)")
                } else {
                    Button("Hide") { Task { await connection.hideWorktree(worktree) } }
                        .tint(.gray)
                        .accessibilityIdentifier("hide-\(worktree.task)")
                }
            }
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
