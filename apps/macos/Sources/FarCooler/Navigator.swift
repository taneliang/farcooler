import AgentKit
import SwiftUI

// The workspace navigator (ov-92): the board, as a source list on the left of
// the main area, in three sections, Orchestrator, Tasks and Worktrees. What
// is selected in it is what the main area shows: the orchestrator by
// default, a task, or a worktree. The orchestrator's rail and its popped-open
// panel are gone; it's a row here instead, which says what it's doing.
//
// Worked out here, as values, so the order ↑ and ↓ walk, each section's
// membership and what the orchestrator's row says are the ones
// `NavigatorTests` pins.

/// One row of the navigator ↑ and ↓ step through.
enum NavigatorItem: Hashable {
    case orchestrator
    case task(String)
    case worktree(String)
}

enum Navigator {
    typealias Selection = ContentView.Selection

    /// The rows top to bottom, section by section: the orchestrator's,
    /// the tasks the list shows, then the Worktrees section's. Only the
    /// loose worktrees are rows of their own, a task's being named on its
    /// task's row; and the orchestrator is never a worktree, nor its task.
    static func items(orchestrator: Bool, tasks: [String], worktrees: [String]) -> [NavigatorItem] {
        (orchestrator ? [.orchestrator] : []) + tasks.map(NavigatorItem.task) + worktrees.map(NavigatorItem.worktree)
    }

    /// The row `selection` lights in board `board`'s navigator: the task
    /// open, or the one a worktree open was opened from (`trail`); a
    /// worktree open whole; else, at the workspace's own level, the
    /// orchestrator. Nil for another board's, or Needs You.
    static func current(_ selection: Selection?, trail: Selection?, board: String) -> NavigatorItem? {
        if let task = WorkspaceNavigation.selectedTask(selection, trail: trail, board: board) { return .task(task) }
        switch selection {
        case .workspace(_, board, nil)?: return .orchestrator
        case .workspace(_, board, .worktree(let id, _)?)?, .looseWorktree(_, let id, _)?: return .worktree(id)
        default: return nil
        }
    }

    /// The row `by` on from `current` in `items`, held at either end; from
    /// none, or one not in the list (a task's worktree, opened whole), the
    /// first going down and the last going up. Nil with nothing to go to.
    static func step(from current: NavigatorItem?, by: Int, in items: [NavigatorItem]) -> NavigatorItem? {
        guard !items.isEmpty else { return nil }
        guard let current, let at = items.firstIndex(of: current) else { return by >= 0 ? items.first : items.last }
        return items[min(max(at + by, 0), items.count - 1)]
    }
}

/// The orchestrator's row at the top of the navigator: who it is, its state
/// as an icon and a word, what it's doing now, and how many tasks are in
/// progress (ov-92).
enum OrchestratorRow {
    enum State: Equatable {
        /// No orchestrator runs this workspace.
        case none
        case starting
        case working
        /// It's asking you something.
        case needsYou
        /// It finished a turn nobody has seen.
        case unread
        case idle
        /// Its pane exited, was lost, or can't be read.
        case stopped
    }

    /// Its state, from its seat alone: the tasks waiting on you are on their
    /// own rows below it, and the header counts them.
    static func state(seat: BoardPane?) -> State {
        guard let seat else { return .none }
        let status = seat.terminal.status
        if status == .blocked { return .needsYou }
        if ConversationColumn.unread(seat) { return .unread }
        switch status {
        case .starting: return .starting
        case .working: return .working
        case .lost, .exited, .failed, .failedRun, .unreadable: return .stopped
        default: return .idle
        }
    }

    /// The state's word, beside its icon.
    static func word(_ state: State) -> String {
        switch state {
        case .none: "No Orchestrator"
        case .starting: "Starting"
        case .working: "Working"
        case .needsYou: "Needs You"
        case .unread: "Done"
        case .idle: "Idle"
        case .stopped: "Stopped"
        }
    }

    /// The one line under its name: what it's doing now, from what the
    /// runner already sends for its pane. The question it's blocked on;
    /// working, its hook-reported activity or plan position (`line`), else
    /// the last thing it said; finished or idle, the last thing it said,
    /// else "Idle since 3:42 PM". Nil with nothing to say.
    static func nowDoing(
        _ terminal: Terminal?, state: State,
        time: (Date) -> String = { $0.formatted(date: .omitted, time: .shortened) }
    ) -> String? {
        guard let terminal else { return nil }
        func text(_ s: String?) -> String? {
            guard let s = s?.trimmingCharacters(in: .whitespacesAndNewlines), !s.isEmpty else { return nil }
            return s
        }
        switch state {
        case .none, .starting, .stopped:
            return nil
        case .needsYou:
            return text(terminal.blockedQuestion) ?? text(terminal.line)
        case .working:
            return text(terminal.line) ?? text(terminal.said) ?? text(terminal.feed?.last)
        case .idle, .unread:
            if let said = text(terminal.said) { return said }
            guard let since = terminal.activitySince else { return nil }
            return "Idle since \(time(Date(timeIntervalSince1970: since / 1000)))"
        }
    }

    /// "3 tasks in progress", or nil with none.
    static func inProgress(_ count: Int) -> String? {
        switch count {
        case ...0: nil
        case 1: "1 task in progress"
        default: "\(count) tasks in progress"
        }
    }

    static func accessibilityLabel(agent: String?, state: State) -> String {
        guard state != .none else { return "No Orchestrator" }
        return ["Orchestrator", agent, word(state)].compactMap { $0 }.joined(separator: ", ")
    }
}

/// What the window hands the navigator for its orchestrator row.
struct NavigatorOrchestrator {
    var state: OrchestratorRow.State
    /// Its harness: claude, codex.
    var agent: String?
    var nowDoing: String?
    var offers: [ConversationColumn.Offer] = []
    var candidates: [BoardPane] = []
    var onSelect: () -> Void = {}
    var onStart: (OrchestratorHarness) -> Void = { _ in }
    var onUse: (BoardPane) -> Void = { _ in }
}

/// The orchestrator's row: selected, it's what the main area shows.
struct OrchestratorRowView: View {
    let model: NavigatorOrchestrator
    let inProgress: Int
    let selected: Bool
    /// The navigator has the keyboard: selected reads in the accent.
    let keyed: Bool

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 0) {
            icon
                .frame(width: ColumnGrid.step, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                if model.state == .none {
                    Text(OrchestratorRow.word(.none))
                        .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                    actions
                } else {
                    HStack(spacing: 6) {
                        Text(model.agent ?? "Orchestrator")
                            .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                            .lineLimit(1)
                        Text(OrchestratorRow.word(model.state))
                            .font(.system(size: WorkspaceStyle.PaneText.secondary))
                            .foregroundStyle(model.state == .needsYou ? Color.accentColor : Color.secondary)
                            .lineLimit(1)
                    }
                    if let doing = model.nowDoing {
                        Text(doing)
                            .font(.system(size: WorkspaceStyle.PaneText.secondary))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .help(doing)
                            .accessibilityIdentifier("orchestrator-now-doing")
                    }
                    if let count = OrchestratorRow.inProgress(inProgress) {
                        Text(count)
                            .font(.system(size: WorkspaceStyle.PaneText.minimum))
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigatorRow(selected: selected, keyed: keyed)
        .onTapGesture(perform: model.onSelect)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(OrchestratorRow.accessibilityLabel(agent: model.agent, state: model.state))
        .accessibilityValue(model.nowDoing ?? "")
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction(.default, model.onSelect)
        .accessibilityIdentifier("navigator-orchestrator")
    }

    /// Its state as an icon: a spinner while it works or starts, the accent
    /// dot when it needs you or has news, else its glyph, dimmed when
    /// there's nothing running.
    @ViewBuilder
    private var icon: some View {
        switch model.state {
        case .working, .starting:
            ProgressView().controlSize(.mini).frame(width: 10, height: 10)
        case .needsYou, .unread:
            Circle()
                .fill(model.state == .needsYou ? Color.accentColor : GlancePalette.amber(scheme))
                .frame(width: 7, height: 7)
        case .idle:
            Image(systemName: "person.wave.2").font(.system(size: 10)).foregroundStyle(.secondary)
        case .none, .stopped:
            Image(systemName: "person.wave.2").font(.system(size: 10)).foregroundStyle(.tertiary)
        }
    }

    /// No orchestrator: start one, or take up a running terminal.
    @ViewBuilder
    private var actions: some View {
        let starts = model.offers.compactMap { offer -> OrchestratorHarness? in
            if case .start(let harness) = offer { return harness }
            return nil
        }
        if !starts.isEmpty {
            // Side by side where they fit, else one under the other: the
            // navigator can be as narrow as 240 pt.
            ViewThatFits(in: .horizontal) {
                HStack(spacing: SidebarGrid.gap) { startMenu(starts); useMenu }
                VStack(alignment: .leading, spacing: ColumnGrid.rhythm / 2) { startMenu(starts); useMenu }
            }
            .controlSize(.small)
            .padding(.top, 2)
        }
    }

    private func startMenu(_ starts: [OrchestratorHarness]) -> some View {
        Menu("Start Orchestrator") {
            ForEach(starts) { harness in Button(harness.title) { model.onStart(harness) } }
        }
        .fixedSize()
    }

    @ViewBuilder
    private var useMenu: some View {
        if !model.candidates.isEmpty {
            Menu("Use as Orchestrator…") {
                ForEach(model.candidates, id: \.terminal.id) { pane in
                    Button(pane.terminal.label) { model.onUse(pane) }
                }
            }
            .fixedSize()
        }
    }
}
