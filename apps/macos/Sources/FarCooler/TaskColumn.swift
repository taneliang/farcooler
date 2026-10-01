import AgentKit
import SwiftUI

// A task, drilled into (spec §4.4): the task itself first, whole (its status,
// title, intent, acceptance and record), and its agent and changes beneath,
// behind a divider. A task need not have a worktree or a terminal at all, so
// with neither the space beneath is one line, never a placeholder.
//
// The rules it draws by are values here, `TaskColumnModel`, so
// `TaskColumnTests` pins them.

enum TaskColumnModel {
    /// What the agent half shows.
    enum Agent: Equatable {
        /// The layout holding the task's agent.
        case live
        /// Nobody's working on it. `openWorktree` when the task has a
        /// worktree to open instead.
        case none(openWorktree: Bool)
    }

    static func agent(hasAgent: Bool, worktree: String?) -> Agent {
        hasAgent ? .live : .none(openWorktree: worktree != nil)
    }

    /// The sentence in place of an agent, or nil when there is one.
    static func sentence(_ agent: Agent) -> String? {
        switch agent {
        case .live: return nil
        case .none(openWorktree: true): return "No agent is working on this task."
        case .none(openWorktree: false): return "Nothing has started on this task yet."
        }
    }

    /// The worktree the task's changes and Open Worktree are about: the
    /// task's own (`Task.worktree_id`), else the one its agent works in.
    static func worktree(of row: TaskRow, agent: BoardPane?) -> String? {
        row.worktreeID.flatMap { $0.isEmpty ? nil : $0 } ?? agent?.worktree.id
    }

    /// Whether the changes lead: In Review, a task is its changes; otherwise
    /// its agent is where the work is happening.
    static func changesLead(_ status: TaskStatus) -> Bool { status == .inReview }

    /// The agent's share of the height beneath the task: what the divider was last
    /// dragged to for this kind of task, else the larger share to whichever
    /// leads.
    static func agentShare(status: TaskStatus, stored: Double?) -> Double {
        if let stored, stored > 0 { return min(max(stored, minimumShare), 1 - minimumShare) }
        return changesLead(status) ? 0.35 : 0.65
    }

    static let minimumShare = 0.15

    /// How much room the agent and changes beneath the task get.
    enum Work: Equatable {
        /// One line: there's nothing to draw but a sentence and its
        /// actions. No agent, and no changes to show.
        case compact
        /// Its share of the height, under the divider.
        case full
    }

    static func work(_ agent: Agent, showsChanges: Bool) -> Work {
        agent == .live || showsChanges ? .full : .compact
    }

    /// The task's text's share of the view's height, over its agent and
    /// changes: what the divider was last dragged to, else 40%.
    static func contentShare(stored: Double?) -> Double {
        if let stored, stored > 0 { return min(max(stored, minimumShare), 1 - minimumShare) }
        return 0.4
    }

    /// "1 terminal", "3 terminals", "No terminals".
    static func terminalCount(_ n: Int) -> String {
        switch n {
        case 0: return "No terminals"
        case 1: return "1 terminal"
        default: return "\(n) terminals"
        }
    }
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
    init(row: TaskRow, store: TaskBoardStore) {
        self.init(row: row, store: store) { shown, detail, question in
            TaskCard(
                row: shown, detail: detail, question: question, canAnswer: store.offersWrites,
                onAnswer: { body in await store.answer(row, with: body) },
                draft: TaskCard.Draft(read: { store.draft(for: $0) }, write: { store.setDraft($1, for: $0) }))
        }
    }
}

/// A task's header, over its text: key and title, and the status pop-up.
struct TaskViewHeader: View {
    let row: TaskRow
    @ObservedObject var store: TaskBoardStore

    var body: some View {
        HStack(spacing: 8) {
            Text(row.key)
                .font(.system(size: WorkspaceStyle.PaneText.title, design: .monospaced))
                .foregroundStyle(.secondary)
            Text(row.title)
                .font(.system(size: 13, weight: .semibold))
                .lineLimit(2)
                .truncationMode(.tail)
                .textSelection(.enabled)
            Spacer(minLength: 6)
            // The card's Move To, in the header: the status is the task's.
            Menu(row.status.title) {
                ForEach(TaskStatus.allCases, id: \.self) { status in
                    Button(status.title) { Task { await store.move(row, to: status) } }
                        .disabled(status == row.status)
                }
            }
            .fixedSize()
            .help("Move this task")
            .disabled(!store.offersWrites)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(WorkspaceStyle.canvas)
    }
}

/// The line over a task's agent and changes: its worktree and how many
/// terminals it has, or why there's nothing there, with the agent picker and
/// Open Worktree. With nothing beneath it, it's the whole of that area.
struct TaskWorkHeader: View {
    let agent: TaskColumnModel.Agent
    /// The worktree, without the orchestrators seated in it.
    let worktree: Worktree?
    let agents: [BoardPane]
    let chosen: BoardPane?
    var onChooseAgent: (BoardPane) -> Void
    var onOpenWorktree: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            if let worktree {
                Image(systemName: "arrow.triangle.branch").foregroundStyle(.secondary)
                Text("Worktree \(worktree.task)")
                    .font(.system(size: 12, weight: .semibold))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text(TaskColumnModel.terminalCount(worktree.terminals.count))
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .fixedSize()
            }
            if let sentence = TaskColumnModel.sentence(agent) {
                Text(sentence)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 6)
            if agents.count > 1 {
                Menu {
                    ForEach(Array(zip(agents, BoardPane.titles(agents))), id: \.0.id) { pane, title in
                        Button(title) { onChooseAgent(pane) }
                    }
                } label: {
                    Text("agent · \(chosen.map { Terminal.name(of: $0.terminal.preset) } ?? "")")
                }
                .fixedSize()
                .help("Choose which agent to show")
            } else if let chosen {
                Text("agent · \(Terminal.name(of: chosen.terminal.preset))")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .fixedSize()
            }
            if worktree != nil {
                Button("Open Worktree", action: onOpenWorktree)
                    .controlSize(.small)
                    .fixedSize()
                    .help("Show this task’s worktree full size, with all its layouts")
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 30)
        .background(WorkspaceStyle.canvas)
    }
}

/// A task's text over its agent and changes, with a divider between them
/// whose place is remembered per window. With nothing beneath but a line
/// (`.compact`), the text takes the rest; with Focus, the work takes all of
/// it.
struct TaskViewSplit<Content: View, WorkArea: View>: View {
    let work: TaskColumnModel.Work
    /// Focus (⌃⌘↩): the agent and changes at full height.
    let focused: Bool
    @ViewBuilder let content: () -> Content
    @ViewBuilder let workArea: () -> WorkArea

    @SceneStorage("task.split.content") private var contentShare: Double = 0

    var body: some View {
        switch work {
        case .compact:
            VStack(spacing: 0) {
                content().frame(maxHeight: .infinity)
                Divider()
                workArea()
            }
        case .full where focused:
            workArea()
        case .full:
            ShareSplit(stored: $contentShare, fallback: TaskColumnModel.contentShare(stored: nil)) {
                content()
            } bottom: {
                workArea()
            }
        }
    }
}

/// The agent over the changes, with a divider between them whose place is
/// remembered per window, one for tasks in review and one for the rest.
/// Either alone when the other isn't there.
struct TaskColumnSplit<Agent: View, Changes: View>: View {
    let status: TaskStatus
    let showsChanges: Bool
    let showsAgent: Bool
    @ViewBuilder let agent: () -> Agent
    @ViewBuilder let changes: () -> Changes

    @SceneStorage("task.split.review") private var reviewShare: Double = 0
    @SceneStorage("task.split.work") private var workShare: Double = 0

    var body: some View {
        if showsChanges && showsAgent {
            let lead = TaskColumnModel.changesLead(status)
            ShareSplit(
                stored: lead ? $reviewShare : $workShare,
                fallback: TaskColumnModel.agentShare(status: status, stored: nil)
            ) {
                agent()
            } bottom: {
                changes()
            }
            .id(lead)
        } else if showsChanges {
            changes()
        } else {
            agent()
        }
    }
}

/// Two views, one over the other, with a divider between them dragged to
/// set the top's share of the height, kept in `stored` once dropped.
struct ShareSplit<Top: View, Bottom: View>: View {
    @Binding var stored: Double
    /// The top's share before anything is stored.
    let fallback: Double
    @ViewBuilder let top: () -> Top
    @ViewBuilder let bottom: () -> Bottom

    @State private var dragging: Double?
    @State private var space = UUID()

    var body: some View {
        GeometryReader { proxy in
            let share = dragging ?? (stored > 0 ? Self.clamped(stored) : fallback)
            let height = proxy.size.height
            VStack(spacing: 0) {
                top().frame(height: max(0, height * share - 3))
                divider(height: height)
                bottom().frame(maxHeight: .infinity)
            }
            .coordinateSpace(name: space)
        }
    }

    static func clamped(_ share: Double) -> Double {
        min(max(share, TaskColumnModel.minimumShare), 1 - TaskColumnModel.minimumShare)
    }

    private func divider(height: CGFloat) -> some View {
        Rectangle()
            .fill(Color.primary.opacity(0.12))
            .frame(height: 1)
            .padding(.vertical, 2.5)
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
            // A pointer style rather than pushing and popping `NSCursor`,
            // which a drag ending outside the divider left stuck.
            .pointerStyle(.rowResize)
            .gesture(
                DragGesture(minimumDistance: 1, coordinateSpace: .named(space))
                    .onChanged { value in
                        guard height > 0 else { return }
                        dragging = Self.clamped(Double(value.location.y / height))
                    }
                    .onEnded { _ in
                        if let dragging { stored = dragging }
                        dragging = nil
                    })
            .accessibilityHidden(true)
    }
}

/// Going up and down the levels (spec §4.4): Open Worktree drills from a
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
    /// name one by its id.
    static func crumbs(
        _ selection: Selection?, trail: Selection?, workspace: String,
        task: (String) -> String, worktree: (String) -> String
    ) -> [Crumb] {
        guard case .workspace(let host, let id, let focus?)? = selection else { return [] }
        let top = Crumb(title: workspace, target: .workspace(host: host, workspace: id, focus: nil))
        switch focus {
        case .task(let t):
            return [top, Crumb(title: task(t), target: nil)]
        case .worktree(let wt, _):
            let here = Crumb(title: worktree(wt), target: nil)
            if case .workspace(host, id, .task(let t)?)? = back(from: selection, trail: trail) {
                return [top, Crumb(title: task(t), target: trail), here]
            }
            return [top, here]
        }
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
    /// from (a task or a worktree opened, Focus, or the orchestrator popped
    /// open, which `focusColumn` stands for), and nobody else wanting it.
    static func goesBack(responder: NSResponder?, selection: ContentView.Selection?, focusColumn: Bool) -> Bool {
        guard !keepsEscape(responder) else { return false }
        return focusColumn || selection?.focus != nil
    }
}

/// Which window a view is in, kept weakly: what Esc as Back checks, so an
/// Esc in Settings doesn't close a task in the window behind it.
@MainActor
final class WindowBox {
    weak var window: NSWindow?
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
