import AgentKit
import SwiftUI

// A workspace's task column (spec §4.4): a task's card in its header, its
// agent, and its changes, beside the board it's on.
//
// It replaces the modal card sheet. The rules it draws by are values here,
// `TaskColumnModel`, so `TaskColumnTests` pins them.

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

    /// The worktree the column's changes and Open Worktree are about: the
    /// task's own (`Task.worktree_id`), else the one its agent works in.
    static func worktree(of row: TaskRow, agent: BoardPane?) -> String? {
        row.worktreeID.flatMap { $0.isEmpty ? nil : $0 } ?? agent?.worktree.id
    }

    /// Whether the changes lead: In Review, a task is its changes; otherwise
    /// its agent is where the work is happening.
    static func changesLead(_ status: TaskStatus) -> Bool { status == .inReview }

    /// The agent's share of the column's height: what the divider was last
    /// dragged to for this kind of task, else the larger share to whichever
    /// leads.
    static func agentShare(status: TaskStatus, stored: Double?) -> Double {
        if let stored, stored > 0 { return min(max(stored, minimumShare), 1 - minimumShare) }
        return changesLead(status) ? 0.35 : 0.65
    }

    static let minimumShare = 0.15

    /// Whether the card starts expanded: in Needs Decision, where its
    /// question and Answer buttons are the point.
    static func startsExpanded(_ status: TaskStatus) -> Bool { status == .needsDecision }
}

/// The task column's card: `row`'s record and question, drawn from the
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

/// The task column's header: key and title, the status pop-up, the agent
/// picker, Open Worktree and close, with the card under a disclosure.
struct TaskColumnHeader: View {
    let row: TaskRow
    @ObservedObject var store: TaskBoardStore
    let agents: [BoardPane]
    let chosen: BoardPane?
    let worktree: String?
    @Binding var expanded: Bool
    var onChooseAgent: (BoardPane) -> Void
    var onOpenWorktree: () -> Void
    var onClose: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Button {
                    expanded.toggle()
                } label: {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                        .frame(width: 14, height: 14)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help(expanded ? "Hide the card" : "Show the card")
                .accessibilityLabel(expanded ? "Hide the Card" : "Show the Card")
                Text(row.key)
                    .font(.system(size: WorkspaceStyle.PaneText.title, design: .monospaced))
                    .foregroundStyle(.secondary)
                Text(row.title)
                    .font(.system(size: 13, weight: .semibold))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 6)
                // Replaces the card's Move To: the status is the card's, and
                // the header is where the card is now.
                Menu(row.status.title) {
                    ForEach(TaskStatus.allCases, id: \.self) { status in
                        Button(status.title) { Task { await store.move(row, to: status) } }
                            .disabled(status == row.status)
                    }
                }
                .fixedSize()
                .help("Move this task")
                .disabled(!store.offersWrites)
                Button(action: onClose) {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.borderless)
                .help("Close the task (⌃⌘←)")
                .accessibilityLabel("Close Task")
            }
            HStack(spacing: 8) {
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
                }
                Spacer(minLength: 0)
                if worktree != nil {
                    Button("Open Worktree", action: onOpenWorktree)
                        .controlSize(.small)
                        .help("Show this task’s worktree and all its layouts")
                }
            }
            .padding(.top, 4)
            .padding(.leading, 22)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(WorkspaceStyle.canvas)
    }
}

/// The agent over the changes, with a divider between them whose place is
/// remembered per window, one for tasks in review and one for the rest.
struct TaskColumnSplit<Agent: View, Changes: View>: View {
    let status: TaskStatus
    let showsChanges: Bool
    @ViewBuilder let agent: () -> Agent
    @ViewBuilder let changes: () -> Changes

    @SceneStorage("task.split.review") private var reviewShare: Double = 0
    @SceneStorage("task.split.work") private var workShare: Double = 0
    @State private var dragging: Double?

    private var stored: Double { TaskColumnModel.changesLead(status) ? reviewShare : workShare }

    var body: some View {
        if showsChanges {
            GeometryReader { proxy in
                let share = dragging ?? TaskColumnModel.agentShare(status: status, stored: stored)
                let height = proxy.size.height
                VStack(spacing: 0) {
                    agent().frame(height: max(0, height * share - 3))
                    divider(height: height, share: share)
                    changes().frame(maxHeight: .infinity)
                }
                .coordinateSpace(name: "task.split")
            }
        } else {
            agent()
        }
    }

    private func divider(height: CGFloat, share: Double) -> some View {
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
                DragGesture(minimumDistance: 1, coordinateSpace: .named("task.split"))
                    .onChanged { value in
                        guard height > 0 else { return }
                        let next = min(
                            max(Double(value.location.y / height), TaskColumnModel.minimumShare),
                            1 - TaskColumnModel.minimumShare)
                        dragging = next
                    }
                    .onEnded { _ in
                        if let dragging {
                            if TaskColumnModel.changesLead(status) { reviewShare = dragging } else { workShare = dragging }
                        }
                        dragging = nil
                    })
            .accessibilityHidden(true)
    }
}

/// A task's agent half when nobody's working on it.
struct TaskColumnNoAgent: View {
    let agent: TaskColumnModel.Agent
    var onOpenWorktree: () -> Void

    var body: some View {
        VStack(spacing: 10) {
            Text(TaskColumnModel.sentence(agent) ?? "")
                .font(.callout)
                .foregroundStyle(.secondary)
            if agent == .none(openWorktree: true) {
                Button("Open Worktree", action: onOpenWorktree)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Going to a task's worktree and back (spec §4.4): Open Worktree replaces
/// the task column with the worktree's own layouts, and Back returns along
/// the breadcrumb.
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
    /// worktree from the Worktrees disclosure included, drops it.
    static func keeps(trail: Selection?, opened: String?, now: Selection?) -> Bool {
        guard case .workspace(let host, let id, .task?)? = trail, let opened else { return false }
        if case .workspace(host, id, .worktree(opened, _)?)? = now { return true }
        return false
    }

    /// Where Back goes: along the breadcrumb to the task a worktree was
    /// opened from, while the window is still in that worktree; else the
    /// third column closes. Nil when there's nothing to go back from.
    static func back(from selection: Selection?, trail: Selection?) -> Selection? {
        guard case .workspace(let host, let id, let focus?)? = selection else { return nil }
        if case .worktree = focus, case .workspace(host, id, .task?)? = trail { return trail }
        return .workspace(host: host, workspace: id, focus: nil)
    }
}

/// An opened worktree's header in the third column: the breadcrumb back to
/// its task when it was opened from one, and its name and close otherwise.
struct OpenedWorktreeHeader: View {
    /// The task it was opened from, as "bil-9", or nil.
    let task: String?
    let worktree: String
    var onBack: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            if let task {
                Button(action: onBack) {
                    Image(systemName: "chevron.left")
                }
                .buttonStyle(.borderless)
                .help("Back to \(task) (⌃⌘←)")
                .accessibilityLabel("Back")
                Button(task, action: onBack)
                    .buttonStyle(.plain)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(.secondary)
                Text("›").foregroundStyle(.tertiary)
            }
            Text(worktree).font(.system(size: 12, weight: .semibold)).lineLimit(1)
            Spacer(minLength: 0)
            if task == nil {
                Button(action: onBack) { Image(systemName: "xmark") }
                    .buttonStyle(.borderless)
                    .help("Close the worktree (⌃⌘←)")
                    .accessibilityLabel("Close Worktree")
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 30)
        .background(WorkspaceStyle.canvas)
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
    /// from (a third column, or Focus Column), and nobody else wanting it.
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

/// A task column's changes: today's Changes view, reused as it is (ruling
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
