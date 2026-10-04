import SwiftUI

// A workspace's board, on a phone: the list form, in-line on the workspace
// screen's Board segment (ov-55, spec §5), and the card rows it and the task
// screen share.
//
// A list grouped by status rather than the Mac's kanban: seven columns side
// by side do not fit a phone, and the order is the Mac's — Needs Decision
// first, because it is the one status waiting on the person reading. Every
// sentence and every rule on a card is AgentKit's (`TaskBoardModel`,
// `TaskBoardAgents`, `BoardForm`), so this phone and the Mac say the same
// thing about the same card. This file draws.
//
// It used to be a sheet over the shell's overview, and an Agent button closed
// it to land on the pane. A card now pushes its task (`TaskScreen`), and an
// agent pushes over that, so the board is never covered or dismissed by a
// jump (spec §6.3).

/// A pane a card can go to, as the board draws it.
struct BoardAgent: Identifiable, Equatable {
    /// The terminal's id, which is what `ShellScreen` resolves to a tab.
    let id: String
    /// The menu item's words: `claude in fix-reconnect`, told apart from a
    /// twin by its short id. See `TaskAgentLink.menuTitles`.
    let title: String
    /// The pane's own mark, so an agent waiting on a question is amber here
    /// too — the card says which agent needs you before you go.
    let mark: GlanceMark
}

/// A workspace's board, in-line on its screen: every status, Needs Decision
/// first, each a section with its count (spec §5's list form).
///
/// An empty status is a header reading "Backlog 0" that doesn't open: a list
/// that dropped it said nothing about what isn't there, and moved every
/// heading under it when a task was filed. Done and Canceled start
/// collapsed, and what's collapsed is remembered per workspace on this
/// device (`BoardForm.collapsed`). A phone is always the list.
struct WorkspaceBoardList: View {
    /// The board as last read, or nil when it has never been read.
    let board: TaskBoardModel?
    /// Whether the last read failed. With a board in hand, that board is
    /// still drawn and a line says it may be behind.
    let unread: Bool
    /// The runner and workspace, for where the collapsed sections are kept.
    let place: PhoneWorkspace
    /// Whether this runner can be believed about its panes right now — see
    /// `TaskAgentLink.speaksOfAgents`. False draws no agent control at all.
    let speaksOfAgents: Bool
    /// How many tasks are waiting on the person: the workspace's decision
    /// items once the runner's list is read, the column until then
    /// (`RunnerBoards.waiting`). The line the Mac's board header draws.
    let waiting: Int
    /// The panes working a card, in fleet order.
    let agents: (TaskRow) -> [BoardAgent]
    /// The pane a card's subagents live in, for its Subagent control.
    let orchestrator: (TaskRow) -> BoardAgent?
    let onOpen: (TaskRow) -> Void
    let onJump: (BoardAgent) -> Void
    let onRefresh: () async -> Void
    /// Whether this workspace has an orchestrator to plan its work (not
    /// Main), which is what an empty board points at (ov-184).
    let ledByOrchestrator: Bool
    /// Whether one is up to tell, which decides what a blank board says and
    /// whether it offers Show Orchestrator (ov-205).
    let orchestratorRunning: Bool
    /// Switches the workspace to its Orchestrator segment.
    let onShowOrchestrator: (() -> Void)?
    /// A finished status's History page, pushed (ov-103).
    let onHistory: (TaskStatus) -> Void

    @State private var collapsed: Set<TaskStatus>
    /// What's been read on this board, on this phone (ov-104): what Done
    /// keeps (`BoardDone`). Read again whenever the list comes back.
    @State private var reads: BoardReads
    /// The long sections showing every task, not just ten.
    @State private var showingMore: Set<TaskStatus> = []

    init(
        board: TaskBoardModel?, unread: Bool, place: PhoneWorkspace, speaksOfAgents: Bool,
        waiting: Int, agents: @escaping (TaskRow) -> [BoardAgent],
        orchestrator: @escaping (TaskRow) -> BoardAgent? = { _ in nil },
        onOpen: @escaping (TaskRow) -> Void,
        onJump: @escaping (BoardAgent) -> Void, onRefresh: @escaping () async -> Void,
        ledByOrchestrator: Bool = false, orchestratorRunning: Bool = true,
        onShowOrchestrator: (() -> Void)? = nil, onHistory: @escaping (TaskStatus) -> Void = { _ in }
    ) {
        self.board = board
        self.unread = unread
        self.place = place
        self.speaksOfAgents = speaksOfAgents
        self.waiting = waiting
        self.agents = agents
        self.orchestrator = orchestrator
        self.onOpen = onOpen
        self.onJump = onJump
        self.onRefresh = onRefresh
        self.ledByOrchestrator = ledByOrchestrator
        self.orchestratorRunning = orchestratorRunning
        self.onShowOrchestrator = onShowOrchestrator
        self.onHistory = onHistory
        _collapsed = State(
            initialValue: BoardForm.collapsed(host: place.runner, workspace: place.workspace))
        _reads = State(initialValue: PhoneReads.load(place))
    }

    var body: some View {
        ZStack {
            if let board, BoardForm.isBlank(board), !unread {
                // Seven headers each reading zero is a blank page. Show the
                // shape of what will appear, say who fills it, and, with no
                // orchestrator to tell, the way to start one: the orchestrator
                // owns the task list (ov-184), so there's no Add Task here.
                ContentUnavailableView {
                    Label(FirstRunCopy.Phone.boardTitle, systemImage: "checklist")
                } description: {
                    VStack(spacing: 16) {
                        TaskSkeleton().frame(maxWidth: 240)
                        Text(
                            BoardForm.blankLine(
                                ledByOrchestrator: ledByOrchestrator,
                                orchestratorRunning: orchestratorRunning))
                    }
                } actions: {
                    if BoardForm.offersOrchestrator(
                        ledByOrchestrator: ledByOrchestrator, orchestratorRunning: orchestratorRunning),
                        let onShowOrchestrator
                    {
                        Button(FirstRunCopy.Phone.showOrchestrator, action: onShowOrchestrator)
                            .buttonStyle(.bordered)
                            .accessibilityIdentifier("board-show-orchestrator")
                    }
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("board-empty")
            } else if let board {
                list(board)
            } else if unread {
                ContentUnavailableView {
                    Label("Couldn’t Read This Board", systemImage: "exclamationmark.triangle")
                } description: {
                    Text("Far Cooler couldn’t read this board. Pull down to try again.")
                }
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .refreshable { await onRefresh() }
        .onAppear { reads = PhoneReads.load(place) }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("board")
    }

    private func waitingPill(_ text: String) -> some View {
        Text(text)
            .font(.footnote.weight(.semibold))
            .lineLimit(1)
            .fixedSize()
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(Color.accentColor.opacity(0.18), in: Capsule())
    }

    private func list(_ board: TaskBoardModel) -> some View {
        List {
            if unread {
                Section {
                    Label(
                        "Couldn’t read this board just now. This is how it was last read.",
                        systemImage: "exclamationmark.triangle"
                    )
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                }
            }
            // The one count worth putting above a board, in the Mac's words
            // (`TaskBoardModel.waitingSentence`), and nothing at zero: a
            // badge reading zero teaches people to ignore it. One line
            // always; where the sentence doesn't fit it's said short, as the
            // Mac says it.
            if let sentence = TaskBoardModel.waitingSentence(waiting) {
                Section {
                    ViewThatFits(in: .horizontal) {
                        waitingPill(sentence)
                        waitingPill("\(waiting) waiting")
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .listRowBackground(Color.clear)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(sentence)
                    .accessibilityIdentifier("board-waiting")
                }
            }
            ForEach(board.sections.filter { BoardForm.canExpand($0) }) { section in
                let open = BoardForm.isExpanded(section, collapsed: collapsed)
                Section {
                    if open {
                        let cut = section.cut(
                            reads: reads, showingAll: showingMore.contains(section.status), now: Date())
                        ForEach(cut.rows) { row in
                            let live = speaksOfAgents ? agents(row) : []
                            TaskBoardCardRow(
                                row: row,
                                live: live,
                                orchestrator: orchestrator(row),
                                speaksOfAgents: speaksOfAgents,
                                presence: row.agentPresence(
                                    livePanes: live.count, runnerRecordsTasks: speaksOfAgents),
                                onOpen: { onOpen(row) },
                                onJump: onJump)
                        }
                        if cut.hidden > 0 {
                            Button(BoardSectionCut.showMoreTitle(cut.hidden)) {
                                withAnimation { _ = showingMore.insert(section.status) }
                            }
                            .font(.footnote)
                            .accessibilityIdentifier("board-show-more-\(section.id)")
                        } else if showingMore.contains(section.status), !section.status.isFinished,
                            section.rows.count > BoardSectionCut.limit
                        {
                            Button("Show Fewer") {
                                withAnimation { _ = showingMore.remove(section.status) }
                            }
                            .font(.footnote)
                            .accessibilityIdentifier("board-show-fewer-\(section.id)")
                        }
                        if let total = cut.history {
                            // "All Done  94 ›": the History page, pushed.
                            Button { onHistory(section.status) } label: {
                                HStack {
                                    Text(BoardDone.historyTitle(section.status))
                                    Spacer()
                                    Text("\(total)")
                                        .monospacedDigit()
                                        .foregroundStyle(.tertiary)
                                    Image(systemName: "chevron.forward")
                                        .font(.caption.weight(.semibold))
                                        .foregroundStyle(.tertiary)
                                }
                                .contentShape(.rect)
                            }
                            .buttonStyle(.plain)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .accessibilityLabel("\(BoardDone.historyTitle(section.status)), \(total)")
                            .accessibilityIdentifier("board-history-\(section.id)")
                        }
                    }
                } header: {
                    header(section, open: open)
                }
            }
            // The statuses with nothing in them, once, rather than a dim header
            // each.
            if let note = BoardForm.emptyNote(board) {
                Section {
                    Text(note)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("board-empty-statuses")
                }
            }
            // Rows this build has no status for: carried and shown under a
            // heading that says it is this app that is behind, never dropped
            // and never filed under a status they are not in.
            if !board.unreadable.isEmpty {
                Section("Not On This Version") {
                    ForEach(board.unreadable) { row in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(row.key)
                                .font(.system(.caption, design: .monospaced))
                                .foregroundStyle(.secondary)
                            Text(row.title)
                            Text(row.status)
                                .font(.system(.caption2, design: .monospaced))
                                .foregroundStyle(.tertiary)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
    }

    /// "In Progress 3", with a chevron when it opens. An empty status's
    /// header has no chevron and does nothing: there's nothing under it.
    private func header(_ section: TaskBoardColumn, open: Bool) -> some View {
        let expandable = BoardForm.canExpand(section)
        return Button {
            guard expandable else { return }
            if collapsed.contains(section.status) {
                collapsed.remove(section.status)
            } else {
                collapsed.insert(section.status)
            }
            BoardForm.setCollapsed(collapsed, host: place.runner, workspace: place.workspace)
        } label: {
            HStack(spacing: PaneMetrics.tight) {
                // The same face and weight for the name and its count, and
                // `.secondary` for an empty status: `.disabled` dimmed it to
                // about a quarter contrast and left a big title beside a tiny
                // monospaced digit.
                Text(section.title)
                    .foregroundStyle(expandable ? .primary : .secondary)
                Spacer()
                // Trailing, tertiary, in tabular digits, as the Mac's
                // headers count (ov-104).
                Text("\(section.count)")
                    .font(.subheadline.monospacedDigit())
                    .foregroundStyle(.tertiary)
                if expandable {
                    Image(systemName: "chevron.forward")
                        .font(.caption.weight(.semibold))
                        .rotationEffect(.degrees(open ? 90 : 0))
                        .foregroundStyle(.tertiary)
                }
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .allowsHitTesting(expandable)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(section.title) \(section.count)")
        .accessibilityValue(expandable ? (open ? "Expanded" : "Collapsed") : "Empty")
        .accessibilityIdentifier("board-section-\(section.id)")
    }
}

/// What's been read on a workspace's board on this phone (ov-104): the Mac's
/// rule (`BoardReads`), kept in this phone's defaults by runner and
/// workspace. A task opened is read (`TaskScreen`).
enum PhoneReads {
    static func load(_ place: PhoneWorkspace, now: Date = Date()) -> BoardReads {
        DefaultsBoardReads().load(host: place.runner, workspace: place.workspace, now: now)
    }

    static func open(_ row: TaskRow, latest: Date?, place: PhoneWorkspace, now: Date = Date()) {
        var reads = load(place, now: now)
        reads.open(row, latest: latest, now: now)
        DefaultsBoardReads().save(reads, host: place.runner, workspace: place.workspace)
    }
}

/// One card: its key, its title, what it asks of you, how long it has sat, how
/// much of its acceptance holds, and the way to its agent.
struct TaskBoardCardRow: View {
    let row: TaskRow
    let live: [BoardAgent]
    /// Where its subagents live, when it has some and that pane is known.
    var orchestrator: BoardAgent?
    /// Whether the runner can be believed about its agents right now.
    var speaksOfAgents = true
    let presence: TaskAgentPresence
    let onOpen: () -> Void
    let onJump: (BoardAgent) -> Void

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(alignment: .top, spacing: PaneMetrics.step) {
            // The card's words are the way into it, and the Agent control
            // beside them is its own button. Two buttons in one row and not a
            // `NavigationLink` row: a link makes the whole row one control, and
            // the Agent button inside it would stop being one.
            Button(action: onOpen) {
                words
            }
            .buttonStyle(.plain)
            // One element for the card's words, and the control beside it its
            // own: an identifier on the `HStack` would be pushed down onto the
            // Agent button too and rename it.
            .accessibilityElement(children: .combine)
            .accessibilityHint("Opens the task")
            .accessibilityIdentifier("board-card-\(row.key)")

            AgentControl(
                key: row.key, live: live, orchestrator: orchestrator, presence: presence,
                onJump: onJump)
        }
        .padding(.vertical, 2)
    }

    private var words: some View {
        VStack(alignment: .leading, spacing: PaneMetrics.tight) {
            HStack(spacing: PaneMetrics.tight) {
                // Mono, because a key is typed into a terminal, and the
                // brief's rule is that mono means data.
                Text(row.key)
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.secondary)
                // On the board's tick, like the sentence below it: a card
                // crosses a day of silence on a quiet board, with no data
                // change to redraw it, and its icon and sentence turn together.
                BoardTick { now in
                    if row.staleness(at: now) == .stale {
                        Image(systemName: "clock")
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                            .accessibilityHidden(true)
                    }
                }
            }
            Text(row.title)
                .font(.subheadline)
                .lineLimit(3)
            if let ask = row.callToAction {
                // Amber, because Needs Decision is the one status waiting
                // on the person reading — which is what amber means here.
                Text(ask)
                    .font(.caption.weight(.medium))
                    .foregroundStyle(GlancePalette.amber(scheme))
            }
            CardStartLines(row: row, speaksOfAgents: speaksOfAgents)
            CardTimeLines(row: row, timeFont: .caption2)
            if let progress = row.acceptanceProgress {
                AcceptanceLine(progress: progress)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(.rect)
    }
}

/// What holds a card back and when it starts: "Waiting on ov-191 and ov-192"
/// in amber, because a block is the one thing here that needs attention, then
/// the quiet start line ("Waiting to build, 2nd in line", "Claude subagent
/// working, 12 min"). Both are AgentKit's sentences, and the Mac and Android
/// say the same ones. Redrawn on the minute, because the line counts minutes.
struct CardStartLines: View {
    let row: TaskRow
    var speaksOfAgents = true

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        BoardTick { now in
            if let blocked = row.blockedSummary {
                Text(blocked)
                    .font(.caption.weight(.medium))
                    .foregroundStyle(GlancePalette.amber(scheme))
                    .accessibilityIdentifier("board-blocked-\(row.key)")
            }
            if let line = row.startLine(at: now, speaksOfAgents: speaksOfAgents) {
                Text(line)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("board-start-\(row.key)")
            }
        }
    }
}

/// A card's two clock sentences, redrawn on the minute.
///
/// "Hasn’t moved in 3 days", or else "Updated 2h ago" / "Added 3d ago" —
/// both composed by AgentKit against the moment handed in. The board redraws
/// only when a task changes, so without a clock of its own a card filed at six
/// would still read "Added just now" at eleven. `BoardTick` scoped to these
/// lines alone, so a tick redraws the sentences and not the board.
struct CardTimeLines: View {
    let row: TaskRow
    let timeFont: Font

    var body: some View {
        BoardTick { now in
            if let note = row.stalenessNote(at: now) {
                Text(note)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else if let time = row.timeNote(at: now) {
                Text(time)
                    .font(timeFont)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// `2 of 5`, or `All 5 met` in the accent color once every line holds.
struct AcceptanceLine: View {
    let progress: TaskAcceptanceProgress

    var body: some View {
        // An `HStack` and not a `Label`: inside a `List` a label's icon gets
        // the row's icon column, which set the words a thumb's width away from
        // the glyph they belong to.
        HStack(spacing: 3) {
            Image(systemName: progress.isComplete ? "checkmark.circle.fill" : "checkmark.circle")
            Text(progress.sentence).monospacedDigit()
        }
        .accessibilityElement(children: .ignore)
        .font(.caption.weight(progress.isComplete ? .medium : .regular))
        .foregroundStyle(progress.isComplete ? Color.accentColor : Color.secondary)
        .accessibilityLabel("Acceptance: \(progress.sentence)")
    }
}

/// The way from a card to the agent working it: a button for one, a menu for
/// several, "Subagent" into the orchestrator's pane, a quiet "No Agent" for a
/// task in progress with nobody on it, and nothing on a runner that cannot say.
struct AgentControl: View {
    let key: String
    let live: [BoardAgent]
    /// Where the subagents live, for `.subagents`; nil makes that plain text.
    var orchestrator: BoardAgent?
    let presence: TaskAgentPresence
    let onJump: (BoardAgent) -> Void

    var body: some View {
        switch presence {
        case .unsaid:
            EmptyView()
        case .noAgent:
            Text(presence.title ?? "")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .accessibilityIdentifier("board-no-agent-\(key)")
        case .subagents:
            // The orchestrator's pane, where the subagents live. Not a button
            // when that pane isn't known: a control that leads nowhere is
            // worse than a word.
            if let orchestrator {
                Button {
                    onJump(orchestrator)
                } label: {
                    pill(mark: orchestrator.mark, trailing: "arrow.forward")
                }
                .buttonStyle(.bordered)
                .buttonBorderShape(.capsule)
                .controlSize(.small)
                .accessibilityLabel("Go to Orchestrator")
                .accessibilityHint(orchestrator.title)
                .accessibilityIdentifier("board-subagent-\(key)")
            } else {
                Text(presence.title ?? "")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityIdentifier("board-subagent-\(key)")
            }
        case .agents:
            if live.count == 1, let only = live.first {
                Button {
                    onJump(only)
                } label: {
                    pill(mark: only.mark, trailing: "arrow.forward")
                }
                .buttonStyle(.bordered)
                .buttonBorderShape(.capsule)
                .controlSize(.small)
                .accessibilityLabel("Go to Agent")
                .accessibilityHint(only.title)
                .accessibilityIdentifier("board-agent-\(key)")
            } else {
                Menu {
                    ForEach(live) { agent in
                        Button {
                            onJump(agent)
                        } label: {
                            Text(agent.title)
                        }
                    }
                } label: {
                    pill(mark: live.first?.mark, trailing: "chevron.down")
                }
                .buttonStyle(.bordered)
                .buttonBorderShape(.capsule)
                .controlSize(.small)
                .accessibilityLabel(presence.title ?? "Agents")
                .accessibilityIdentifier("board-agent-\(key)")
            }
        }
    }

    private func pill(mark: GlanceMark?, trailing: String) -> some View {
        HStack(spacing: PaneMetrics.tight) {
            if let mark { ShellMarkView(mark: mark, size: 7) }
            Text(presence.title ?? "")
            Image(systemName: trailing)
                .font(.caption2.weight(.bold))
        }
        .font(.caption.weight(.medium))
    }
}
