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
    /// A line of Unread, by its id (`BoardSummaryStrip.lines`): its task,
    /// chosen there (ov-177).
    case unread(String)
    case task(String)
    case worktree(String)

    /// The task it opens: a task row's, or an Unread line's.
    var taskID: String? {
        switch self {
        case .task(let id): id
        case .unread(let line): BoardSummaryStrip.task(ofLine: line)
        case .orchestrator, .worktree: nil
        }
    }
}

enum Navigator {
    typealias Selection = ContentView.Selection

    /// The rows top to bottom, section by section: the orchestrator's,
    /// Unread's lines, the tasks the list shows, then the Worktrees section's. Only the
    /// loose worktrees are rows of their own, a task's being named on its
    /// task's row; and the orchestrator is never a worktree, nor its task.
    static func items(
        orchestrator: Bool, unread: [String] = [], tasks: [String], worktrees: [String]
    ) -> [NavigatorItem] {
        (orchestrator ? [.orchestrator] : []) + unread.map(NavigatorItem.unread) + tasks.map(NavigatorItem.task)
            + worktrees.map(NavigatorItem.worktree)
    }

    /// The row lit for `current`, given the Unread line the selection was
    /// chosen at (`line`): that line while its task is still the one
    /// selected, so the selection stays where it was clicked (ov-177).
    static func place(_ current: NavigatorItem?, line: String?) -> NavigatorItem? {
        guard let line, let current, case .task(let id) = current, BoardSummaryStrip.task(ofLine: line) == id
        else { return current }
        return .unread(line)
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
    /// the last thing it said (`lastSaid`); finished or idle, the last thing
    /// it said, else "Idle since 3:42 PM" ("Idle since Tue 3:42 PM" before
    /// today). Nil with nothing to say.
    ///
    /// A `line` that's only the runner's headline ("claude 4m", "claude
    /// needs you": what `feed::line` falls back to with no signal) says
    /// nothing the state word doesn't, and counts as none (ov-92 review).
    static func nowDoing(
        _ terminal: Terminal?, state: State, now: Date = Date(), calendar: Calendar = .current,
        time: (Date, _ today: Bool) -> String = { date, today in
            today
                ? date.formatted(date: .omitted, time: .shortened)
                : date.formatted(.dateTime.weekday(.abbreviated).hour().minute())
        }
    ) -> String? {
        guard let terminal else { return nil }
        func text(_ s: String?) -> String? {
            guard let s = s?.trimmingCharacters(in: .whitespacesAndNewlines), !s.isEmpty else { return nil }
            return s
        }
        let signal = text(terminal.line).flatMap { isHeadline($0, agent: Terminal.name(of: terminal.preset)) ? nil : $0 }
        switch state {
        case .none, .starting, .stopped:
            return nil
        case .needsYou:
            return text(terminal.blockedQuestion) ?? signal
        case .working:
            return signal ?? text(terminal.lastSaid)
        case .idle, .unread:
            if let said = text(terminal.lastSaid) { return said }
            guard let since = terminal.activitySince else { return nil }
            let date = Date(timeIntervalSince1970: since / 1000)
            return "Idle since \(time(date, calendar.isDate(date, inSameDayAs: now)))"
        }
    }

    /// Whether `line` is only the runner's headline for `agent`: its name and
    /// a state word, or its name and how long its turn has run ("claude 4m",
    /// "claude 1h 5m", "claude 12s"). See `farcooler_core::feed::headline`.
    static func isHeadline(_ line: String, agent: String) -> Bool {
        let prefix = agent + " "
        guard line.hasPrefix(prefix) else { return false }
        let rest = String(line.dropFirst(prefix.count))
        if ["working", "needs you", "done", "idle", "failed"].contains(rest) { return true }
        return rest.wholeMatch(of: /\d+[hms]( \d+[ms])?/) != nil
    }

    /// The row's title, the one place the navigator names it: there's only
    /// ever one orchestrator, so it's a row, not a section under a header
    /// (ov-177).
    static let title = "Orchestrator"

    /// The row's quiet last line: its agent, then how many tasks are in
    /// progress, "claude · 3 tasks in progress"; nil with neither.
    static func foot(agent: String?, inProgress count: Int) -> String? {
        let parts = [agent, inProgress(count)].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
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
    /// Its pane's status, for the agent status mark the rest of the app
    /// draws (`StatusGlyph`); nil with no pane.
    var status: Status?
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
            // In the glyph column, centered under the carets, so its title is
            // on the text column with every other row's (`NavigatorGrid`).
            icon
                .glyphColumn()
                .gridMark("orchestrator", .icon)
            VStack(alignment: .leading, spacing: 2) {
                if model.state == .none {
                    Text(OrchestratorRow.word(.none))
                        .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                        .gridMark("orchestrator", .text)
                    actions
                } else {
                    HStack(spacing: 6) {
                        // What it is, on the row itself: there's no section
                        // header over it to say so (ov-177).
                        Text(OrchestratorRow.title)
                            .font(.system(size: WorkspaceStyle.PaneText.body, weight: .medium))
                            .lineLimit(1)
                            .gridMark("orchestrator", .text)
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
                    if let foot = OrchestratorRow.foot(agent: model.agent, inProgress: inProgress) {
                        Text(foot)
                            .font(.system(size: WorkspaceStyle.PaneText.minimum))
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .navigatorRow(selected: selected, keyed: keyed, leading: 0, box: "orchestrator")
        .onTapGesture(perform: model.onSelect)
        // With no orchestrator its two menus stay reachable on their own.
        .accessibilityElement(children: model.state == .none ? .contain : .combine)
        .accessibilityLabel(OrchestratorRow.accessibilityLabel(agent: model.agent, state: model.state))
        .accessibilityValue(model.nowDoing ?? "")
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction(.default, model.onSelect)
        .accessibilityIdentifier("navigator-orchestrator")
    }

    /// Its state as an icon: the app's own agent status mark while it
    /// works or starts (ov-177: never the system's spinner), the
    /// accent dot when it needs you or has news, else its glyph, dimmed when
    /// there's nothing running.
    @ViewBuilder
    private var icon: some View {
        switch model.state {
        case .working, .starting:
            StatusGlyph(status: model.status ?? (model.state == .starting ? .starting : .working))
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

/// The filter field atop the navigator (⌘F, ov-103), and the History
/// page's search: a rounded field, its glyph (the filter's funnel lines, or
/// search's magnifying glass) in it, and a clear button while it holds
/// something. Esc clears it; on an empty field, Esc leaves it
/// (`onLeave`), as the sidebar's search does (`SearchEscape`).
///
/// Laid out on the navigator's grid (`NavigatorGrid`, ov-177): one row tall,
/// its box reaching `NavigatorGrid.outset` past the grid's edge, its glyph
/// centered in the glyph column (under the carets), and its text on the text
/// column, where every row's text starts.
struct NavigatorFilterField: View {
    @Binding var text: String
    var focused: FocusState<Bool>.Binding
    var placeholder = "Filter"
    var glyph = "line.3.horizontal.decrease"
    let onLeave: () -> Void

    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        HStack(spacing: 0) {
            Image(systemName: glyph)
                .font(.system(size: 10))
                .foregroundStyle(.tertiary)
                .glyphColumn()
                .gridMark("filter", .icon)
            TextField(placeholder, text: $text)
                .textFieldStyle(.plain)
                .font(.system(size: WorkspaceStyle.PaneText.body))
                .focused(focused)
                .onExitCommand {
                    let next = SearchEscape.after(query: text)
                    text = next.query
                    if !next.keepsFocus { onLeave() }
                }
                .accessibilityIdentifier("navigator-filter")
                .gridMark("filter", .text)
            if !text.isEmpty {
                Button { text = "" } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .help("Clear Filter")
                .accessibilityLabel("Clear Filter")
            }
        }
        .padding(.trailing, ColumnGrid.rhythm)
        .frame(height: ColumnGrid.rowHeight)
        .background {
            RoundedRectangle.control.fill(Fill.inset(contrast))
                .boxOutset()
                .gridMark("filter", .box)
        }
        .overlay {
            RoundedRectangle.control
                .strokeBorder(
                    focused.wrappedValue ? Color.accentColor.opacity(0.6) : WorkspaceStyle.hairline,
                    lineWidth: focused.wrappedValue ? 1 : 0.5)
                .boxOutset()
        }
        .help("Filter tasks by key or title (⌘F)")
    }
}
