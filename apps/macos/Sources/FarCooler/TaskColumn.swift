import AgentKit
import SwiftUI

// A task, opened beside the navigator (spec §4.4, ov-98): a compact header
// that stays put (key, title, the agent's state, its status), and
// under it four full-height tabs, Overview (the task itself, whole), Agent
// (its terminal), Changes (its diff) and Files (its worktree's files,
// ov-189). A task need not have a worktree or a terminal at all; then
// Agent, Changes and Files say so and offer Ask the Orchestrator.
//
// The rules it draws by are values here, `TaskColumnModel` and
// `TaskTabMemory`, so `TaskColumnTests` and `TaskTabsTests` pin them.

enum TaskColumnModel {
    /// What the Agent tab shows.
    enum Agent: Equatable {
        /// The layout holding the task's agent.
        case live
        /// Nobody's working on it. `openWorktree` when the task has a
        /// worktree to open instead.
        case none(openWorktree: Bool)
        /// No worktree of its own, and its agent's pane is there but
        /// exited: something did start. The breadcrumb offers the worktree
        /// it ran in (`WorkspaceWorktrees.worktrees(of:host:in:)`, ov-185).
        case stopped
    }

    /// `stopped` when an agent pane opened for the task is still listed
    /// (`hadAgent(_:host:in:)`) though none is working it.
    static func agent(hasAgent: Bool, worktree: String?, stopped: Bool = false) -> Agent {
        if hasAgent { return .live }
        if worktree == nil && stopped { return .stopped }
        return .none(openWorktree: worktree != nil)
    }

    /// Whether `host` lists an agent's pane opened for task `id`, live or
    /// exited: the breadcrumb's pane rule, agents only. Never an orchestrator,
    /// whatever task id it carries.
    static func hadAgent(_ id: String, host: String, in fleet: Fleet) -> Bool {
        fleet.worktrees.contains { worktree in
            (worktree.host ?? "") == host
                && worktree.terminals.contains { !$0.isOrchestrator && $0.runsAgent && $0.taskId == id }
        }
    }

    /// The sentence in place of an agent, or nil when there is one.
    static func sentence(_ agent: Agent) -> String? {
        switch agent {
        case .live: return nil
        case .none(openWorktree: true): return "No agent is working on this task."
        case .none(openWorktree: false): return "Nothing has started on this task yet."
        case .stopped: return "This task’s agent has stopped."
        }
    }

    /// The worktree the task's changes and Open Worktree are about: the
    /// task's own (`Task.worktree_id`), else the one its agent works in.
    static func worktree(of row: TaskRow, agent: BoardPane?) -> String? {
        row.worktreeID.flatMap { $0.isEmpty ? nil : $0 } ?? agent?.worktree.id
    }

    /// What the Agent tab draws.
    enum AgentView: Equatable {
        /// No agent: why not, and Ask the Orchestrator.
        case start
        /// A task passed on the way, glancing: nothing mounted until it
        /// settles.
        case waiting
        /// The layout holding the agent.
        case tiled
        /// The agent's terminal alone, in no layout read yet.
        case bare
    }

    /// The Agent tab's view, from the agent, the glance and the layout, and
    /// never from which tab is in front, so a tab switch can't swap one
    /// view for another and re-wrap the terminal.
    static func agentView(hasAgent: Bool, settled: Bool, hasLayout: Bool) -> AgentView {
        guard hasAgent else { return .start }
        guard settled else { return .waiting }
        return hasLayout ? .tiled : .bare
    }

    /// Whether the task's diff has the Diff menu's keys: it was clicked
    /// into (`focus`), and Changes is in front.
    static func changesFocused(focus: String?, task: String, tab: TaskTab) -> Bool {
        focus == task && tab == .changes
    }

    /// "1 terminal", "3 terminals", "No terminals".
    static func terminalCount(_ n: Int) -> String {
        switch n {
        case 0: return "No terminals"
        case 1: return "1 terminal"
        default: return "\(n) terminals"
        }
    }

    /// What the task view's New Terminal button does (ov-234): open the
    /// task's worktree whole first, so Back returns to the task, then make a
    /// plain shell in it. Nil, so no button, when the task has no worktree or
    /// its runner can't answer.
    static func newTerminalAction(
        in lane: Worktree?, refused: Bool,
        openWorktree: @escaping () -> Void,
        newTerminal: @escaping (Worktree) -> Void
    ) -> (() -> Void)? {
        guard let lane, !refused else { return nil }
        return {
            openWorktree()
            newTerminal(lane)
        }
    }

    /// The agent's state in the header, "claude working", or nil with no
    /// agent: its name and the navigator's own state word
    /// (`OrchestratorRow`), so the two can't come to say it differently.
    static func agentLine(_ pane: BoardPane?) -> (text: String, needsYou: Bool, working: Bool)? {
        guard let pane else { return nil }
        let state = OrchestratorRow.state(seat: pane)
        let name = Terminal.name(of: pane.terminal.preset)
        return (
            "\(name) \(OrchestratorRow.word(state).lowercased())", state == .needsYou,
            state == .working || state == .starting
        )
    }
}

/// A task's tabs (ov-98): the task itself, its agent's terminal, its diff,
/// and its worktree's files (ov-189).
enum TaskTab: String, CaseIterable, Identifiable {
    case overview, agent, changes, files

    var id: Self { self }

    var title: String {
        switch self {
        case .overview: "Overview"
        case .agent: "Agent"
        case .changes: "Changes"
        case .files: "Files"
        }
    }
}

/// Which tab each task shows, per window: the one last chosen for it, else
/// Agent while an agent is working on it and Overview otherwise.
///
/// Only a choice is kept, never a default, so a task opened before its agent
/// started opens on Agent once one has.
struct TaskTabMemory: Equatable {
    private(set) var chosen: [String: TaskTab] = [:]

    /// The tab a task opens on with nothing chosen for it.
    static func initial(agentWorking: Bool) -> TaskTab { agentWorking ? .agent : .overview }

    func tab(for task: String, agentWorking: Bool) -> TaskTab {
        chosen[task] ?? Self.initial(agentWorking: agentWorking)
    }

    mutating func choose(_ tab: TaskTab, for task: String) { chosen[task] = tab }

    /// ⌃⌘] (`offset` 1) and ⌃⌘[ (-1): the next or previous tab from the
    /// one `task` shows, wrapping, and chosen.
    mutating func step(_ task: String, by offset: Int, agentWorking: Bool) {
        let all = TaskTab.allCases
        let at = all.firstIndex(of: tab(for: task, agentWorking: agentWorking)) ?? 0
        choose(all[((at + offset) % all.count + all.count) % all.count], for: task)
    }
}

/// The task view's type (ov-98): one ladder, title, section label, body,
/// metadata, and prose at a reading measure, on the 8 pt rhythm
/// (`ColumnGrid.rhythm`).
enum TaskTypography {
    /// The task's title, in the header.
    static let title = Font.title2.weight(.semibold)
    /// A section's label: Intent, Acceptance, Record.
    static let label = Font.subheadline.weight(.semibold)
    /// What's written: intent, acceptance lines, notes.
    static let body = Font.body
    /// Who and when, counts, the quiet lines.
    static let meta = Font.caption

    /// The body's point size, for the views that take a size.
    static var bodySize: CGFloat { NSFont.preferredFont(forTextStyle: .body).pointSize }
    /// The metadata's point size.
    static var metaSize: CGFloat { NSFont.preferredFont(forTextStyle: .caption1).pointSize }

    /// About seventy characters of body text, the reading measure: measured
    /// on a sentence of the body's own type, and rounded to the rhythm.
    static let measure: CGFloat = {
        let sample = "How vexingly quick daft zebras jump, and the five boxing wizards jump quickly."
        let font = NSFont.preferredFont(forTextStyle: .body)
        let perCharacter = (sample as NSString).size(withAttributes: [.font: font]).width / CGFloat(sample.count)
        return (70 * perCharacter / ColumnGrid.rhythm).rounded() * ColumnGrid.rhythm
    }()

    /// Between two sections.
    static let sectionGap: CGFloat = 3 * ColumnGrid.rhythm
    /// From a section's label to what it heads.
    static let labelGap: CGFloat = ColumnGrid.rhythm
    /// Between two notes in the record.
    static let noteGap: CGFloat = 2 * ColumnGrid.rhythm
    /// From a note's quiet line to its body.
    static let noteLineGap: CGFloat = ColumnGrid.rhythm / 2
    /// The overview's margins.
    static let inset = EdgeInsets(
        top: 2 * ColumnGrid.rhythm, leading: 3 * ColumnGrid.rhythm, bottom: 3 * ColumnGrid.rhythm,
        trailing: 3 * ColumnGrid.rhythm)
}

/// A task's card: `row`'s record and question, drawn from the
/// board's store, which this observes.
///
/// Its own view for that observation. Built in `ContentView`'s body, which
/// doesn't observe the board, the card of a task just switched to stayed
/// blank, no question and no Answer buttons, until something unrelated
/// redrew the window. Through `detail(for:)` and `question(for:)`, as ov-65
/// had it, so no frame draws another task's record. `card` is what draws
/// them: `TaskCard`, or a test's probe.
struct TaskColumnCard<Card: View>: View {
    let row: TaskRow
    @ObservedObject var store: TaskBoardStore
    let card: (_ row: TaskRow, _ detail: TaskDetailModel, _ question: TaskQuestion?) -> Card

    var body: some View {
        card(
            store.opened?.id == row.id ? store.opened ?? row : row, store.detail(for: row.id),
            store.question(for: row.id))
    }
}

extension TaskColumnCard where Card == TaskCard {
    init(
        row: TaskRow, store: TaskBoardStore, orchestrator: BoardPane? = nil,
        speaksOfAgents: Bool = true, onGoTo: @escaping (BoardPane) -> Void = { _ in }
    ) {
        self.init(row: row, store: store) { shown, detail, question in
            TaskCard(
                row: shown, detail: detail, question: question, canAnswer: store.canAnswer(row.id),
                usage: store.usage(for: row.id), orchestrator: orchestrator, onGoTo: onGoTo,
                speaksOfAgents: speaksOfAgents, onRetryUsage: { store.retryUsage(row) },
                onAnswer: { body in await store.answer(row, with: body) },
                draft: TaskCard.Draft(read: { store.draft(for: $0) }, write: { store.setDraft($1, for: $0) }))
        }
    }
}

/// A task's header, which never scrolls away: key and title, the agent's
/// state, its status, and Ask the Orchestrator.
struct TaskViewHeader: View {
    let row: TaskRow
    var ask: AskOrchestrator.Action = .unavailable
    /// The agent working it, if any (`TaskColumnModel.agentLine`).
    var agent: BoardPane?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: ColumnGrid.rhythm) {
            Text(row.key)
                .font(TaskTypography.meta.monospaced())
                .foregroundStyle(.secondary)
                .fixedSize()
            Text(row.title)
                .font(TaskTypography.title)
                .lineLimit(1)
                .truncationMode(.tail)
                .textSelection(.enabled)
                .help(row.title)
            Spacer(minLength: ColumnGrid.rhythm)
            if let line = TaskColumnModel.agentLine(agent) {
                HStack(spacing: ColumnGrid.rhythm / 2) {
                    if line.working, let agent {
                        // The app's agent status mark, never the system's
                        // spinner (ov-177).
                        StatusGlyph(status: agent.terminal.status)
                    } else if line.needsYou {
                        Circle().fill(Color.accentColor).frame(width: 6, height: 6)
                    }
                    Text(line.text)
                        .font(TaskTypography.meta)
                        .foregroundStyle(line.needsYou ? Color.accentColor : Color.secondary)
                }
                .fixedSize()
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("task-agent-state")
            }
            // The status is the orchestrator's to change: read here, not set.
            Text(row.status.title)
                .font(TaskTypography.meta)
                .foregroundStyle(.secondary)
                .fixedSize()
                .accessibilityIdentifier("task-status")
            AskOrchestratorButton(row: row, action: ask, small: true)
                .fixedSize()
        }
        // The overview's own margin, so the key sits over its text.
        .padding(.horizontal, TaskTypography.inset.leading)
        // Six rhythms: a title2 line with a rhythm and a half each side.
        .frame(maxWidth: .infinity, minHeight: 6 * ColumnGrid.rhythm, alignment: .leading)
        .background(WorkspaceStyle.canvas)
    }
}

/// The bar under the header: the three tabs, and the task's worktree on the
/// right with how many terminals it has, the agent or a picker when several
/// are on it, and Open Worktree.
struct TaskTabBar: View {
    let tab: TaskTab
    var onChoose: (TaskTab) -> Void
    /// The worktree, without the orchestrators seated in it.
    let worktree: Worktree?
    let agents: [BoardPane]
    let chosen: BoardPane?
    var onChooseAgent: (BoardPane) -> Void = { _ in }
    var onOpenWorktree: () -> Void = {}
    /// A shell in the task's worktree (ov-234); nil where the runner can't
    /// take one.
    var onNewTerminal: (() -> Void)?

    var body: some View {
        HStack(spacing: ColumnGrid.rhythm) {
            Picker("Tab", selection: Binding(get: { tab }, set: onChoose)) {
                ForEach(TaskTab.allCases) { tab in Text(tab.title).tag(tab) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .controlSize(.small)
            .fixedSize()
            .help("Overview, Agent, Changes, and Files (⌃⌘[ and ⌃⌘])")
            .accessibilityIdentifier("task-tabs")
            Spacer(minLength: ColumnGrid.rhythm)
            if let worktree {
                Image(systemName: "arrow.triangle.branch")
                    .font(TaskTypography.meta)
                    .foregroundStyle(.secondary)
                Text(worktree.task)
                    .font(ColumnHeader.font(.medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text(TaskColumnModel.terminalCount(worktree.terminals.count))
                    .font(TaskTypography.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize()
            }
            if agents.count > 1 {
                Menu {
                    ForEach(Array(zip(agents, BoardPane.titles(agents))), id: \.0.id) { pane, title in
                        Button(title) { onChooseAgent(pane) }
                    }
                } label: {
                    Text("agent · \(chosen.map { Terminal.name(of: $0.terminal.preset) } ?? "")")
                }
                .controlSize(.small)
                .fixedSize()
                .help("Choose which agent to show")
            }
            if worktree != nil, let onNewTerminal {
                Button("New Terminal", action: onNewTerminal)
                    .controlSize(.small)
                    .fixedSize()
                    .help("Open a terminal in this task’s worktree")
                    .accessibilityLabel("Open Terminal in This Task’s Worktree")
                    .accessibilityIdentifier("task-new-terminal")
            }
            if worktree != nil {
                Button("Open Worktree", action: onOpenWorktree)
                    .controlSize(.small)
                    .fixedSize()
                    .help("Show this task’s worktree full size, with all its layouts")
            }
        }
        .padding(.horizontal, TaskTypography.inset.leading)
        .columnHeader()
    }
}

/// What the Agent and Changes tabs show with nothing to draw: why, in a
/// sentence. Compact, near the top: never a placeholder the height of the
/// view. Starting an agent is the orchestrator's, so there's nothing to press
/// but Ask the Orchestrator.
struct TaskStartPanel: View {
    let sentence: String
    /// The task the sentence is about, for Ask the Orchestrator; nil beside a
    /// sentence that isn't about starting (an old runner's).
    var row: TaskRow?
    var ask: AskOrchestrator.Action = .unavailable

    var body: some View {
        VStack(spacing: 2 * ColumnGrid.rhythm) {
            Text(sentence)
                .font(TaskTypography.body)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            if let row {
                AskOrchestratorButton(row: row, action: ask, small: true)
                    .fixedSize()
            }
        }
        .padding(3 * ColumnGrid.rhythm)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .padding(.top, 4 * ColumnGrid.rhythm)
    }
}

/// The four tabs, full height, one in front.
///
/// Overview and Agent are made once and kept: the one not shown is faded
/// out, takes no click, no keyboard (`outOfSight`) and nothing VoiceOver
/// reaches, so the terminal is never rebuilt and never re-wraps as the tabs
/// switch (`TaskTabsTests`). Changes and Files are each made the first time
/// they're shown and kept from then on, so a task never looked at for its
/// diff never reads one, and one never browsed reads no directory.
struct TaskTabs<Overview: View, Agent: View, Changes: View, Files: View>: View {
    let tab: TaskTab
    @ViewBuilder let overview: () -> Overview
    @ViewBuilder let agent: () -> Agent
    @ViewBuilder let changes: () -> Changes
    @ViewBuilder let files: () -> Files

    @State private var changesShown = false
    @State private var filesShown = false
    /// The pane's own: a task leaving the main area is out of sight whole.
    @Environment(\.outOfSight) private var paneOutOfSight

    var body: some View {
        ZStack(alignment: .topLeading) {
            overview().modifier(TabLayer(shown: tab == .overview, paneOutOfSight: paneOutOfSight))
            agent().modifier(TabLayer(shown: tab == .agent, paneOutOfSight: paneOutOfSight))
            if changesShown || tab == .changes {
                changes().modifier(TabLayer(shown: tab == .changes, paneOutOfSight: paneOutOfSight))
            }
            if filesShown || tab == .files {
                files().modifier(TabLayer(shown: tab == .files, paneOutOfSight: paneOutOfSight))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onChange(of: tab, initial: true) { _, tab in
            if tab == .changes { changesShown = true }
            if tab == .files { filesShown = true }
        }
    }

    /// One tab in the stack: in front, or kept out of sight behind. Out of
    /// sight adds to the pane's (`paneOutOfSight`) and never replaces it:
    /// the front tab of a task leaving the main area lets go of the
    /// keyboard too (ov-98 review M1).
    private struct TabLayer: ViewModifier {
        let shown: Bool
        let paneOutOfSight: Bool

        func body(content: Content) -> some View {
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .opacity(shown ? 1 : 0)
                .allowsHitTesting(shown)
                .accessibilityHidden(!shown)
                .environment(\.outOfSight, paneOutOfSight || !shown)
                .disabled(!shown)
        }
    }
}

/// Going up and down the levels (spec §4.4): Open Worktree goes from a
/// task into its worktree, full size, and Back returns along the
/// breadcrumb.
enum WorkspaceNavigation {
    typealias Selection = ContentView.Selection

    /// Open Worktree from `selection`: the worktree whole, in the same
    /// workspace, and the task it came from as the way back.
    static func openWorktree(_ worktree: String, from selection: Selection) -> (next: Selection, trail: Selection?) {
        guard case .workspace(let host, let id, let focus) = selection else {
            return (selection, nil)
        }
        let next = Selection.workspace(host: host, workspace: id, focus: .worktree(worktree, terminal: nil))
        if case .task? = focus { return (next, selection) }
        return (next, nil)
    }

    /// Whether a breadcrumb back to `trail` still holds on `now`: while the
    /// window is in the worktree Open Worktree opened from it (`opened`),
    /// with any pane of it selected. Choosing anything else, another
    /// worktree from its row under the workspace included, drops it.
    static func keeps(trail: Selection?, opened: String?, now: Selection?) -> Bool {
        guard case .workspace(let host, let id, .task?)? = trail, let opened else { return false }
        if case .workspace(host, id, .worktree(opened, _)?)? = now { return true }
        return false
    }

    /// One step of the breadcrumb over a task or a worktree opened.
    struct Crumb: Equatable {
        var title: String
        /// Where clicking it goes: nil for the level you're at.
        var target: Selection?
    }

    /// The breadcrumb for `selection`: Workspace › Task, Workspace › Task ›
    /// Worktree for one opened from its task (`trail`), or Workspace ›
    /// Worktree. Empty at the workspace's own level. `task` and `worktree`
    /// name one by its id. A project terminal open names itself instead
    /// (`projectTerminal`, its name by worktree and terminal id, or nil for
    /// any other): Workspace › Terminal, in every workspace of the
    /// repository, because it belongs to the project and not to the main
    /// checkout's worktree, which most workspaces don't own (ov-234).
    static func crumbs(
        _ selection: Selection?, trail: Selection?, workspace: String,
        task: (String) -> String, worktree: (String) -> String,
        projectTerminal: (_ worktree: String, _ terminal: String) -> String? = { _, _ in nil }
    ) -> [Crumb] {
        guard case .workspace(let host, let id, let focus?)? = selection else { return [] }
        let top = Crumb(title: workspace, target: .workspace(host: host, workspace: id, focus: nil))
        switch focus {
        case .task(let t):
            return [top, Crumb(title: task(t), target: nil)]
        case .history(let status):
            return [top, Crumb(title: BoardHistory.title(status), target: nil)]
        case .worktree(let wt, let terminal):
            if let terminal, let name = projectTerminal(wt, terminal) {
                return [top, Crumb(title: name, target: nil)]
            }
            let here = Crumb(title: worktree(wt), target: nil)
            if case .workspace(host, id, .task(let t)?)? = back(from: selection, trail: trail) {
                return [top, Crumb(title: task(t), target: trail), here]
            }
            return [top, here]
        }
    }

    /// What one Back does, in order: leave Focus, then go up a level.
    /// Esc and ⌃⌘← (`oneAtATime`) stop after the first that applies; the
    /// breadcrumb's chevron does both. Esc (`toOrchestrator`) goes up to the
    /// orchestrator, past a task a worktree was opened from (ov-92); ⌃⌘←
    /// goes along the breadcrumb.
    struct BackStep: Equatable {
        var leavesFocus = false
        var goesTo: Selection?
    }

    static func backStep(
        focus: Bool, oneAtATime: Bool, toOrchestrator: Bool = false, from selection: Selection?, trail: Selection?
    ) -> BackStep {
        var step = BackStep(leavesFocus: focus, goesTo: nil)
        if focus && oneAtATime { return step }
        step.goesTo = back(from: selection, trail: toOrchestrator ? nil : trail)
        return step
    }

    /// Whether the board keeps the keyboard across a selection change: one
    /// the board asked for (`pending`), within the workspace whose board it
    /// is (⌥⌘2 going up to it, a row chosen or glanced at, a close), where
    /// any other change would take it back.
    static func boardKeepsKeyboard(pending: Bool, from old: Selection?, to new: Selection?) -> Bool {
        guard pending, let old, let new, old != new, old.host == new.host else { return false }
        switch (old, new) {
        case (.workspace(_, let was, _), .workspace(_, let now, _)): return was == now
        // From a loose worktree to its board's workspace, or to another
        // loose worktree or from the workspace to one, in the navigator
        // it's drawn beside.
        case (.looseWorktree, .workspace), (.looseWorktree, .looseWorktree), (.workspace, .looseWorktree): return true
        default: return false
        }
    }

    /// What closing what's opened goes to: the workspace's own level, the
    /// board alone; for a loose worktree, the workspace whose board is beside
    /// it (`board`). Nil with nothing to close.
    static func closing(_ selection: Selection, board: String?) -> Selection? {
        switch selection {
        case .workspace(let host, let id, _?): return .workspace(host: host, workspace: id, focus: nil)
        case .looseWorktree(let host, _, _): return board.map { .workspace(host: host, workspace: $0, focus: nil) }
        default: return nil
        }
    }

    /// A task chosen on its board (ov-85): opened beside the board, or, with
    /// `toggles` (a click), closed when it's the one open already. A glance
    /// (↑ or ↓) only opens.
    static func choosing(
        task: String, host: String, workspace: String, from selection: Selection?, toggles: Bool
    ) -> Selection {
        let open = Selection.workspace(host: host, workspace: workspace, focus: .task(task))
        return toggles && selection == open ? .workspace(host: host, workspace: workspace, focus: nil) : open
    }

    /// The task the board `board` draws selected: the one open beside it,
    /// or the one a worktree open beside it was opened from (`trail`).
    static func selectedTask(_ selection: Selection?, trail: Selection?, board: String) -> String? {
        if case .workspace(_, board, .task(let id)?)? = selection { return id }
        if case .workspace(_, board, .worktree?)? = selection, case .workspace(_, board, .task(let id)?)? = trail {
            return id
        }
        return nil
    }

    /// Where Back goes: along the breadcrumb to the task a worktree was
    /// opened from, while the window is still in that worktree; else up to
    /// the workspace. Nil when there's nothing to go back from.
    static func back(from selection: Selection?, trail: Selection?) -> Selection? {
        guard case .workspace(let host, let id, let focus?)? = selection else { return nil }
        if case .worktree = focus, case .workspace(host, id, .task?)? = trail { return trail }
        return .workspace(host: host, workspace: id, focus: nil)
    }
}

/// Esc as Back (spec §4.9): only when no terminal, and nothing being typed
/// into, has the keyboard, since a terminal needs its Esc and a field uses it
/// to cancel.
@MainActor
enum EscapeBack {
    /// Whether `responder` takes Esc for itself: a terminal, or any text
    /// being edited (a composer, a field, the search box).
    static func keepsEscape(_ responder: NSResponder?) -> Bool {
        responder is TerminalRenderView || responder is NSText || responder is NSTextField
    }

    /// Whether an Esc in the main window goes back: something to go back
    /// from (a task or a worktree selected, a loose one included, or
    /// Focus, which `focusColumn` stands for), and nobody else wanting it.
    /// `closable` is whether what's open has somewhere to close to: a loose
    /// worktree with no board beside it doesn't, and keeps its Esc.
    static func goesBack(
        responder: NSResponder?, selection: ContentView.Selection?, focusColumn: Bool, closable: Bool = true
    ) -> Bool {
        guard !keepsEscape(responder) else { return false }
        if focusColumn { return true }
        if case .looseWorktree? = selection { return closable }
        return selection?.focus != nil
    }
}

/// Which window a view is in, kept weakly: what Esc as Back checks, so an
/// Esc in Settings doesn't close a task in the window behind it.
@MainActor
final class WindowBox {
    weak var window: NSWindow? {
        didSet {
            guard let window, let attached else { return }
            self.attached = nil
            attached(window)
        }
    }
    /// What to do once, when the window is known: a window that took its
    /// record before it was drawn puts its frame back then (ov-233).
    var attached: ((NSWindow) -> Void)?
}

/// Tells a `WindowBox` the window it's drawn in.
struct WindowReader: NSViewRepresentable {
    let box: WindowBox

    func makeNSView(context: Context) -> NSView {
        let view = Reporter()
        view.box = box
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    private final class Reporter: NSView {
        var box: WindowBox?
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            box?.window = window
        }
    }
}

/// A task's changes: today's Changes view, reused as it is (ruling
/// 6), drawn here without a tmux pane, so nothing resizes the agent's window
/// for other clients. It reads the worktree's changes from its store, and
/// nothing it does splits a pane.
struct TaskColumnChanges: View {
    @ObservedObject var changes: ChangesStore
    let isFocused: Bool
    let agents: [ReviewAgentTarget]
    /// The diff was clicked into: the Diff menu's keys are for it now.
    var onFocus: () -> Void = {}

    var body: some View {
        ChangesPane(changes: changes, isFocused: isFocused, agents: agents)
            .simultaneousGesture(TapGesture().onEnded { onFocus() })
    }
}

