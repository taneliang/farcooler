import SwiftUI

// One workspace, pushed from Needs You (spec §6.1): its name as the title.
//
// On an iPhone (ov-616) the screen is the conversation: the orchestrator's
// pane, or Your Rep's for a member (ov-526), full height, its seat named
// under the title. Over it, the two cards (`WorkspaceCards`): Now/Next opens
// the Plan sheet (`PlanSheetScreen`), and Agents opens the agents view
// (ov-619). With no orchestrator started, lost, Restart and Replace; starting,
// "Starting Orchestrator…" and, after 30 seconds, the warning that the seat
// can stick (spec §8).
//
// A workspace with no conversation (a repository's own board on a runner
// without workspaces, or one whose orchestrator was never started) has the
// Plan as its screen, with Start Orchestrator above it where one can be
// started. The orchestrator owns the task list (ov-184): the Plan reads, and
// a decision is answered from its task, but nothing here files, moves or
// edits a task.
//
// On an iPad at regular width the screen is columns instead (ov-348,
// `PadWorkspace.swift`): the tree, the plan and the orchestrator's chat.

struct WorkspaceScreen: View {
    @ObservedObject var fleet: FleetStore
    @ObservedObject var hosts: RunnerStore
    @ObservedObject var connection: Connection
    let place: PhoneWorkspace

    /// An orchestrator this phone asked for, until its pane is up: the
    /// conversation's, wherever Start was pressed.
    @StateObject private var start = OrchestratorStart()
    /// What an iPad's plan column shows (ov-348): kept while the screen is,
    /// across a turn or a resize to the phone's layout and back.
    @State private var canvas: PadCanvas
    /// Whether the tree is shown over the plan, where two columns leave no
    /// room for it as a third.
    @State private var treeShown = false
    /// Whether the tree's column is put away where three fit, as a system
    /// sidebar can be: the plan and the chat take its room.
    @State private var treeCollapsed = false
    /// The screen's width, for `PadLayout`.
    @State private var width: Double = 0
    @Environment(\.horizontalSizeClass) private var sizeClass

    init(fleet: FleetStore, hosts: RunnerStore, connection: Connection, place: PhoneWorkspace) {
        self.fleet = fleet
        self.hosts = hosts
        self.connection = connection
        self.place = place
        _canvas = State(initialValue: WorkspacePlaceMemory.canvas(for: place))
    }

    @Environment(\.phoneNavigator) private var navigator
    @Environment(\.colorScheme) private var scheme

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
        // Who the conversation is with, under the name: Orchestrator, or
        // Your Rep (ov-526), where the segment control said it. Drawn here
        // rather than as the bar's subtitle, whose secondary ink measured
        // 3.4:1 over a light terminal theme's ground; this is primary, and
        // smaller than the name.
        .toolbar {
            if let summary, let seat = subtitle(summary) {
                ToolbarItem(placement: .principal) {
                    VStack(spacing: 0) {
                        Text(title(summary)).font(.headline).lineLimit(1)
                        Text(seat).font(.caption).lineLimit(1)
                    }
                }
            }
        }
        // Its board's and plan's keys preview their tasks (ov-299).
        .environment(\.taskKeyLinker, connection.taskKeyLinker(navigator))
        .onAppear {
            UserDefaults.standard.set(place.stored, forKey: PhoneLaunch.lastWorkspaceKey)
        }
        // Opening this workspace again shows what was last picked here.
        .onChange(of: canvas) { _, shown in WorkspacePlaceMemory.keep(shown, for: place) }
    }

    private var summary: WorkspaceSummary? { connection.workspace(place.workspace) }

    /// The conversation's seat, on a phone whose screen is one.
    private func subtitle(_ summary: WorkspaceSummary) -> String? {
        guard !layout(summary).hasColumns, conversation(summary) else { return nil }
        return Self.talkTitle(in: connection, summary: summary)
    }

    /// Whether the phone's screen is the conversation rather than the Plan.
    private func conversation(_ summary: WorkspaceSummary) -> Bool {
        connection.hasConversation(summary) || start.startedAt != nil
    }

    /// An implicit workspace is called by its repository, as its row was.
    private func title(_ summary: WorkspaceSummary) -> String {
        guard summary.isImplicit else { return summary.name }
        return connection.repositoryNames[summary.id] ?? summary.name
    }

    /// The scheme of what the chrome sits on: the terminal theme's while the
    /// orchestrator's pane is up, which paints the theme's ground, and nothing
    /// to say for the Plan, drawn on the system's own background.
    ///
    /// A member's Talk paints the theme's ground only while their rep's pane
    /// is up (ov-526): its first screen, its notices and its stopped state sit
    /// on the system's own background, and drawn in the theme's scheme the
    /// title and the cards were white on white in light mode.
    private func groundScheme(_ summary: WorkspaceSummary) -> ColorScheme? {
        guard conversation(summary) else { return nil }
        switch connection.talk.seat(for: summary, build: connection.daemon) {
        case .orchestrator: return Themes.shared.current.colorScheme
        case let .rep(_, _, existing):
            let pane = existing.flatMap { rep in
                connection.fleet.worktrees.lazy.flatMap(\.terminals).first { $0.id == rep.terminal }
            }
            guard let pane, !RepSeat.isDown(pane), StateKind.parse(pane.state) != .starting else { return nil }
            return Themes.shared.current.colorScheme
        case .reading, .unknown, .neither: return nil
        }
    }

    /// The layout for this window (`PadLayout`): columns on an iPad at
    /// regular width, the phone's screen otherwise.
    private func layout(_ summary: WorkspaceSummary) -> PadLayout {
        PadLayout.of(
            isPad: Self.isPad, regularWidth: sizeClass == .regular, width: width, implicit: summary.isImplicit)
    }

    private static var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad }

    /// The screen, once its width is known on an iPad. Before that, nothing:
    /// a width of zero is a phone's, and drawing the phone's screen for a
    /// frame flashed it, and started its reads, on every push into a
    /// workspace (review 5). A phone has one layout and needs no width.
    @ViewBuilder
    private func content(_ summary: WorkspaceSummary) -> some View {
        Group {
            if Self.isPad && width == 0 {
                Color.clear
            } else {
                laidOut(summary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onGeometryChange(for: Double.self) { $0.size.width } action: { width = $0 }
    }

    /// One layout for both, so the orchestrator's pane is the same view in
    /// each (`PadWorkspace.swift`): turning an iPad, or narrowing its window,
    /// moves the pane between the screen and a column without building it
    /// again, so its session and its place are kept (ov-66). The columns,
    /// when there are any, come before it.
    @ViewBuilder
    private func laidOut(_ summary: WorkspaceSummary) -> some View {
        let layout = layout(summary)
        #if DEBUG
        let _ = PadProbe.drew(layout, onPad: Self.isPad)
        #endif
        let columns = layout.hasColumns
        let treeColumn = layout == .threeColumns && !treeCollapsed
        // Three columns with the tree put away are two, drawn as two are.
        let widths = (layout == .threeColumns && treeCollapsed ? PadLayout.twoColumns : layout).widths(width)
        let talks = conversation(summary)
        let sidebarUp = layout == .twoColumns && treeShown
        HStack(spacing: 0) {
            if treeColumn {
                tree(summary)
                    .frame(width: widths.tree)
                    .separator(.split, edge: .trailing)
            }
            if columns {
                PadCanvasColumn(connection: connection, summary: summary, place: place, canvas: $canvas) {
                    boardList(summary)
                }
                .frame(width: widths.plan)
                .separator(.split, edge: .trailing)
                // Under the sidebar shown on demand, as the chat is, header
                // and all: VoiceOver reads only the sidebar while it's up.
                .accessibilityHidden(sidebarUp)
            }
            VStack(spacing: 0) {
            if columns {
                PadChatHeader(connection: connection, summary: summary)
                    .accessibilityHidden(sidebarUp)
                // The agents at work, over the chat, where the tray under it
                // was (ov-619): the plan column beside it has Now/Next.
                if !summary.isImplicit {
                    WorkspaceCards(connection: connection, summary: summary, place: place, showsPlan: false)
                        .padding(.top, Spacing.group)
                        .accessibilityHidden(sidebarUp)
                }
            }
            ZStack {
                // The conversation: in columns it's the chat, always up
                // (ov-348); on a phone, the screen, whenever the workspace has
                // one (ov-616). The Plan sheet over it leaves it mounted and
                // live, so opening the Plan and coming back neither reconnects
                // the pane nor loses its place (ov-66).
                if !summary.isImplicit, columns || talks {
                    OrchestratorSegment(connection: connection, summary: summary, place: place, shown: true, start: start)
                        // In columns, a chat's composer is the column's own.
                        .environment(\.composerInColumn, columns)
                        // Under the phone's floating cards (ov-611).
                        .environment(\.fadesUnderBars, !columns)
                        .accessibilityHidden(sidebarUp)
                }
                if !columns, !talks {
                    PhonePlanList(connection: connection, summary: summary, place: place, inSheet: false) {
                        if !summary.isImplicit, connection.talk.seat(for: summary, build: connection.daemon) == .orchestrator {
                            OrchestratorInvitation(connection: connection, summary: summary, start: start)
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            // The chat column, for a test to measure; on a phone, the screen.
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(columns ? "pad-chat" : "workspace-body")
        }
        .overlay(alignment: .leading) {
            if sidebarUp {
                PadTreeSidebar(width: PadLayout.sidebar(width), close: { treeShown = false }) {
                    tree(summary, takesFocus: true)
                }
            }
        }
        .animation(.snappy, value: sidebarUp)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // A bar of the navigation bar's own, so the cards sit in its
        // material and take their touches, rather than under its edge.
        .safeAreaBar(edge: .top) {
            if !columns, talks {
                WorkspaceCards(connection: connection, summary: summary, place: place)
                    // Over the orchestrator's pane the ground is the terminal
                    // theme's, so the text on it follows the theme and not the
                    // phone's appearance (ov-342): black on Nord was 1.6:1.
                    .environment(\.colorScheme, groundScheme(summary) ?? scheme)
            }
        }
        .toolbar {
            if columns {
                // In both column layouts, so its key is there whichever the
                // iPad turned to: in two it shows the tree over the plan, in
                // three it puts the tree's column away and back.
                ToolbarItem(placement: .topBarLeading) {
                    Button {
                        toggleTree(summary)
                    } label: {
                        Label(WorkspaceSegment.tree.title, systemImage: "sidebar.leading")
                    }
                    // The system's Show Sidebar key, so it's in the ⌘ overlay.
                    .keyboardShortcut("s", modifiers: [.command, .control])
                    .accessibilityValue(treeColumn || sidebarUp ? "Shown" : "Hidden")
                    .accessibilityIdentifier("pad-tree-toggle")
                }

                // The board, a toolbar item on an iPad (design §4), shown in
                // the plan column; again, the plan.
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        canvas = canvas == .board ? .plan : .board
                    } label: {
                        Label(WorkspaceSegment.board.title, systemImage: "checklist")
                    }
                    // The Mac's Show Board key (⌘B is the sidebar's there).
                    .keyboardShortcut("b", modifiers: [.command, .shift])
                    .accessibilityAddTraits(canvas == .board ? .isSelected : [])
                    .accessibilityIdentifier("pad-board")
                }
            }
        }
        // The navigation bar's title and Back, over the same ground. Nil
        // elsewhere, so the Plan keeps the system's own.
        .toolbarColorScheme(columns ? nil : groundScheme(summary), for: .navigationBar)
        // What buttons drawn from inside a themed pane take (ov-655): the
        // bar's, which is the system's on an iPad.
        .environment(\.barScheme, columns ? scheme : groundScheme(summary) ?? scheme)
        // The sidebar shown on demand is a passing thing: a new layout puts
        // it away, as a pick does.
        .onChange(of: layout) { _, _ in treeShown = false }
        #if DEBUG
        .overlay(alignment: .bottomTrailing) { layoutProbe(layout) }
        #endif
    }

    /// Show or hide the tree: over the plan in two columns, its own column
    /// in three. The layout is read when the key is pressed, not when the
    /// button was drawn: the ⌃⌘S key kept the closure from before a turn,
    /// and in portrait it put away a column that wasn't there (measured).
    private func toggleTree(_ summary: WorkspaceSummary) {
        if layout(summary) == .threeColumns { treeCollapsed.toggle() } else { treeShown.toggle() }
    }

    /// The tree as an outline, its pick shown in the plan column.
    private func tree(_ summary: WorkspaceSummary, takesFocus: Bool = false) -> some View {
        PadTreeColumn(
            connection: connection, summary: summary, place: place, canvas: canvas, takesFocus: takesFocus
        ) { pick in
            treeShown = false
            switch pick {
            case .canvas(let chosen): canvas = chosen
            case .open(let route): navigator?.open(route)
            case .none: break
            }
        }
    }

    /// The workspace's board, in an iPad's plan column, where the
    /// orchestrator it offers to show is already beside it.
    private func boardList(_ summary: WorkspaceSummary) -> some View {
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
            orchestrator: connection.orchestratorAgent(for:),
            onOpen: { row in canvas = .task(row.id) },
            onJump: { agent in openAgent(agent) },
            onRefresh: { await connection.readBoard(summary) },
            ledByOrchestrator: !summary.isImplicit,
            // A member's board isn't told to start an orchestrator: it's the
            // lead's to run, and they talk through their rep.
            orchestratorRunning: Self.orchestratorIsUp(in: connection, summary: summary)
                || !Self.talksToOrchestrator(in: connection, summary: summary),
            asksRep: connection.talk.seat(for: summary, build: connection.daemon).title == RunnerPeople.repTitle,
            onHistory: { status in navigator?.open(.history(place, status: status.rawValue)) },
            reads: connection.boardReads[summary.id] ?? .firstLook(now: Date()),
            readNotes: { row in await connection.taskRecord(row.id)?.detail.notes },
            readsAreShared: { connection.readsAreShared(workspace: summary.id) },
            onMarkAllRead: { latest in
                guard let rows = connection.boards[summary.id]?.rows else { return }
                connection.markAllRead(rows: rows, latest: latest, workspace: summary.id)
            },
            plan: planHook(summary))
        .task { await connection.readBoard(summary) }
    }

    #if DEBUG
    /// `pad-layout`: which layout is drawn, and how many times an iPad drew
    /// the phone's (`PadProbe`), for a UI test to tell columns from the
    /// phone's screen without measuring.
    private func layoutProbe(_ layout: PadLayout) -> some View {
        Rectangle()
            .fill(Color.white.opacity(0.001))  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
            .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
            .accessibilityElement()
            .accessibilityIdentifier("pad-layout")
            .accessibilityValue(PadProbe.value(layout))
    }
    #endif

    /// The board's plan (ov-274) in an iPad's plan column: a theme or lane
    /// shows there too, and a ruling's Reverse or Discuss stays, since the
    /// chat it went to is beside it (review 8).
    private func planHook(_ summary: WorkspaceSummary) -> PlanBoardHook {
        connection.planHomeHook(summary, place: place, wide: true, onOpen: { canvas = .page($0) }, onRuling: { _ in })
    }

    /// Whether this phone may type to `summary`'s orchestrator: it's the
    /// owner's, or the lead's. A member who isn't the lead talks to their rep
    /// (ov-526), and the runner refuses them the orchestrator (`not_lead`).
    static func talksToOrchestrator(in connection: Connection, summary: WorkspaceSummary) -> Bool {
        connection.talk.seat(for: summary, build: connection.daemon) == .orchestrator
    }

    /// Who this phone's conversation is with: "Orchestrator", or "Your Rep".
    static func talkTitle(in connection: Connection, summary: WorkspaceSummary) -> String {
        connection.talk.seat(for: summary, build: connection.daemon).title
    }

    /// Whether the workspace has an orchestrator that isn't dead, and this
    /// phone may type to it: what every orchestrator composer here asks.
    static func orchestratorIsUp(in connection: Connection, summary: WorkspaceSummary) -> Bool {
        guard talksToOrchestrator(in: connection, summary: summary),
            let terminal = OrchestratorSegment.terminal(in: connection, summary: summary) else { return false }
        switch StateKind.parse(terminal.state) {
        case .lost, .exited, .error: return false
        default: return true
        }
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

// MARK: - Orchestrator

/// The workspace's orchestrator, or what to do about not having one.
struct OrchestratorSegment: View {
    @ObservedObject var connection: Connection
    let summary: WorkspaceSummary
    let place: PhoneWorkspace
    /// Whether it's up, rather than mounted under something.
    let shown: Bool
    /// The start this phone asked for, shared with the Plan's Start
    /// Orchestrator (`OrchestratorInvitation`).
    @ObservedObject var start: OrchestratorStart

    @StateObject private var pastes = ImagePasteQueue()
    @State private var replacing = false
    @Environment(\.screenMargin) private var margin

    /// Which harnesses this runner can start: all of them from a runner that
    /// doesn't say which it has (`DaemonBuild.availability`).
    private var availability: HarnessAvailability {
        connection.daemon?.availability ?? HarnessAvailability(agentsFound: nil)
    }

    var body: some View {
        seatBody
            // Who's on the runner is read with the fleet, and here too for a
            // screen that opens before that read: nothing is typed to the
            // orchestrator until it says this phone may.
            .task { await connection.talk.load(connection) }
    }

    @ViewBuilder
    private var seatBody: some View {
        switch connection.talk.seat(for: summary, build: connection.daemon) {
        case .orchestrator:
            orchestratorBody
        case let .rep(handle, _, existing):
            RepSeat(
                connection: connection, summary: summary, place: place, shown: shown,
                handle: handle, existing: existing)
        case .reading:
            ProgressView()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .accessibilityIdentifier("talk-reading")
        case .unknown:
            ContentUnavailableView {
                Label(RunnerPeople.unknownTitle, systemImage: "exclamationmark.triangle")
            } description: {
                Text(RunnerPeople.unknownBody)
            } actions: {
                Button(RunnerPeople.tryAgain) { Task { await connection.talk.load(connection, force: true) } }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("talk-try-again")
            }
        case .neither:
            ContentUnavailableView {
                Label(RunnerPeople.neitherTitle, systemImage: "person.crop.circle.badge.xmark")
            } description: {
                Text(RunnerPeople.neitherBody)
            }
            .accessibilityIdentifier("talk-neither")
        }
    }

    private var orchestratorBody: some View {
        VStack(spacing: 0) {
            if let refusal = start.refusal {
                Label(refusal, systemImage: "exclamationmark.triangle.fill")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, margin)
                    .padding(.bottom, Spacing.group)
                    .accessibilityIdentifier("orchestrator-refusal")
            }
            state
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                // Images on their way into the orchestrator's terminal, over
                // the pane as a shell pane's are (`ShellScreen`).
                .overlay(alignment: .bottom) { ImagePasteChips(queue: pastes) }
        }
        .confirmationDialog(
            "Replace the Orchestrator?", isPresented: $replacing, titleVisibility: .visible
        ) {
            ForEach(availability.installed, id: \.rawValue) { harness in
                Button("Replace with \(harness.title)", role: .destructive) {
                    start.begin(harness, replace: true, connection: connection, workspace: summary.id)
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This stops the orchestrator that’s running and starts a new one.")
        }
        .onChange(of: orchestrator?.id) { _, id in
            if id != nil { start.startedAt = nil }
        }
        .onChange(of: orchestrator.map { StateKind.parse($0.state) }) { _, kind in
            if let asked = start.asked, start.endedAfter == nil, kind == .lost || kind == .exited || kind == .error {
                start.endedAfter = Date().timeIntervalSince(asked)
            }
        }
    }

    /// The orchestrator's pane, as the fleet has it: the workspace's own
    /// word, else the pane that says it leads this workspace.
    private var orchestrator: Terminal? { Self.terminal(in: connection, summary: summary) }

    static func terminal(in connection: Connection, summary: WorkspaceSummary) -> Terminal? {
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
                starting(since: start.startedAt ?? Date())
            default:
                OrchestratorPane(
                    terminal: terminal, connection: connection, pastes: pastes, place: place,
                    shown: shown)
                    .id(terminal.id)
            }
        } else if let startedAt = start.startedAt {
            starting(since: startedAt)
        } else {
            none
        }
    }

    private var none: some View {
        ContentUnavailableView {
            Label(FirstRunCopy.Phone.orchestratorTitle, systemImage: "person.crop.circle.badge.plus")
        } description: {
            VStack(spacing: 14) {
                PhoneEmptyRows(copy: PhoneEmptyStates.noOrchestrator)
                if availability.isKnown && availability.installed.isEmpty {
                    Text(FirstRunCopy.Conversation.noneInstalled(on: .runner(connection.hostLabel)))
                        .accessibilityIdentifier("orchestrator-none-installed")
                }
            }
        } actions: {
            if mayAct {
                Menu(FirstRunCopy.Phone.start) {
                    ForEach(AgentHarness.allCases, id: \.rawValue) { harness in
                        if availability.isInstalled(harness) {
                            Button(harness.title) { start.begin(harness, replace: false, connection: connection, workspace: summary.id) }
                        } else {
                            Button("\(harness.title) · \(FirstRunCopy.Navigator.notInstalled)") {}
                                .disabled(true)
                        }
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(availability.isKnown && availability.installed.isEmpty)
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

    /// The agent a quick exit 127 says isn't installed, when this phone can
    /// say which: the one it asked for, else the pane's own preset.
    private func missingAgent(_ terminal: Terminal) -> AgentHarness? {
        OrchestratorExit.missingAgent(
            exitCode: terminal.exitCode, endedAfter: start.endedAfter, asked: start.askedHarness,
            preset: terminal.preset)
    }

    @ViewBuilder
    private func lost(_ terminal: Terminal) -> some View {
        if let harness = missingAgent(terminal) {
            notInstalled(harness, terminal)
        } else {
            stopped(terminal)
        }
    }

    private func notInstalled(_ harness: AgentHarness, _ terminal: Terminal) -> some View {
        ContentUnavailableView {
            Label(FirstRunCopy.Conversation.notInstalledTitle(harness), systemImage: "exclamationmark.triangle")
                .accessibilityIdentifier("orchestrator-not-installed")
        } description: {
            Text(FirstRunCopy.Phone.notInstalledBody(harness, on: connection.hostLabel))
        } actions: {
            if mayAct {
                Button(FirstRunCopy.Phone.tryAgain) {
                    start.endedAfter = nil
                    start.asked = Date()
                    Task { await connection.act(.restart, on: terminal) }
                }
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier("orchestrator-try-again")
            }
        }
    }

    private func stopped(_ terminal: Terminal) -> some View {
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
}

/// The orchestrator's own pane, full height: the same terminal and agent
/// views a worktree's panes use, and the one pane being read while it's up.
///
/// **Live only while it's up and nothing covers it** (ov-66 fix): its
/// workspace the top of the stack, with no task pushed over it or in its
/// Plan sheet, and no worktree covering it. Covered, it stays mounted,
/// as ruled, and its session stops (`TerminalView`'s `isVisible`): a
/// stream nobody can see is a phone's battery and data spent for nothing.
///
/// Being read is `TerminalView`'s to claim, as every pane's is: while
/// `isVisible`, it tells `Notifier` and marks the pane seen, and it claims
/// again when it comes back, which is what Back from a cover does. What
/// that claim never does is give itself back when the pane is only hidden,
/// so this does, whenever it stops being live, and when the screen goes.
struct OrchestratorPane: View {
    let terminal: Terminal
    @ObservedObject var connection: Connection
    @ObservedObject var pastes: ImagePasteQueue
    let place: PhoneWorkspace
    let shown: Bool
    /// What a test finds the pane by: the orchestrator's, or a rep's (ov-526).
    var identifier = "orchestrator-pane"

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
            terminal: terminal, connection: connection, pastes: pastes, live: live, identifier: identifier)
    }
}

/// Whether anything is over `place`'s screen on the phone's stack: a
/// screen pushed over it or in its Plan sheet, or a worktree covering the
/// stack.
///
/// A view of its own so that only what it wraps watches the stack: the
/// workspace screen and its segment control never re-render on a push
/// (see `PhoneNavigator`).
private struct CoverReader<Content: View>: View {
    @ObservedObject var navigator: PhoneNavigator
    let place: PhoneWorkspace
    @ViewBuilder let content: (Bool) -> Content

    var body: some View {
        // A screen pushed in the Plan sheet covers it too: the sheet is at
        // its large detent then, and that screen is the one being read.
        content(
            navigator.worktree != nil || navigator.path.last != .workspace(place)
                || navigator.plan?.path.isEmpty == false)
    }
}

/// The orchestrator's terminal, live or not.
private struct OrchestratorTerminal: View {
    let terminal: Terminal
    @ObservedObject var connection: Connection
    @ObservedObject var pastes: ImagePasteQueue
    let live: Bool
    let identifier: String

    #if DEBUG
    /// Which mount of the pane this is, for a UI test to tell a pane kept
    /// mounted from one built again (`orchestrator-mount`).
    @StateObject private var mount = PaneMount()
    #endif

    var body: some View {
        TerminalView(terminal: terminal, isVisible: live, connection: connection, pastes: pastes)
            .modifier(OrchestratorImageDoor(terminal: terminal, connection: connection, pastes: pastes, isVisible: live))
            .background(TerminalPalette.background.ignoresSafeArea(edges: .bottom))
            // A terminal surface in a screen whose chrome is the system's: its
            // own text (the status states) reads against the theme's ground,
            // so the scheme is the theme's from here down and no further.
            .environment(\.colorScheme, Themes.shared.current.colorScheme)
            // One element that contains the pane's, so its conversation's
            // blocks keep their own names (ov-611's margin test reads them).
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(identifier)
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
                    .fill(Color.white.opacity(0.001))  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
                    .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
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

extension Connection {
    /// Decided For You's Keep, Keep All, Reverse and Discuss for `summary`'s
    /// plan, wherever its sections are drawn: the Board's Plan view and the
    /// Plan sheet (ov-300 review 5). `open` goes to the orchestrator's pane
    /// after a send or a draft; the sheet closes itself first.
    func rulingActions(
        _ summary: WorkspaceSummary, place: PhoneWorkspace, open: @escaping @MainActor (PhoneRoute) -> Void
    ) -> PhoneRulingActions {
        let seat = WorkspaceScreen.orchestratorIsUp(in: self, summary: summary)
            ? OrchestratorSegment.terminal(in: self, summary: summary) : nil
        let connection = self
        /// Where the orchestrator's pane is, to go there after a send or a draft.
        @MainActor func goToSeat() {
            guard let seat,
                let home = connection.fleet.worktrees.first(where: { $0.terminals.contains { $0.id == seat.id } })
            else { return }
            open(.worktree(runner: place.runner, worktree: home.id, landing: .terminal(seat.id)))
        }
        return PhoneRulingActions(
            canMark: RulingActions.canMark(runnerTakesMarks: connection.keepsRulingActions, grantedScope: connection.knownBuild?.grantedScope),
            canAsk: seat != nil,
            keep: { ruling in Task { await connection.keepRuling(ruling, in: summary) } },
            keepAll: { Task { await connection.keepAllRulings(in: summary) } },
            reverse: { ruling in
                guard let seat else { return nil }
                let outcome = await RulingActions.reverse(
                    ruling,
                    paste: { await connection.draftPrompt(terminal: seat.id, text: $0) },
                    copy: { UIPasteboard.general.string = $0 })
                if outcome != .copied { goToSeat() }
                return RulingActions.notice(for: outcome, ruling: ruling)
            },
            discuss: { ruling in
                guard let seat else { return nil }
                let delivery = await RulingActions.discuss(
                    ruling,
                    paste: { await connection.draftPrompt(terminal: seat.id, text: $0) },
                    copy: { UIPasteboard.general.string = $0 })
                if delivery != .copied { goToSeat() }
                return delivery == .copied ? "Copied the start of a message about \(ruling.short). Paste it into the orchestrator." : nil
            })
    }
}
