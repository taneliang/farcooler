import AppKit
import AgentKit
import Foundation
import SwiftUI
import Testing

@testable import Far_Cooler

/// A task, drilled into (spec §4.4).
@MainActor
struct TaskColumnTests {
    private static func row(_ status: TaskStatus, worktree: String? = nil) -> TaskRow {
        var row = TaskRow(id: "t-9", key: "bil-9", title: "Invoice PDF export", status: status, statusSince: .now)
        row.worktreeID = worktree
        return row
    }

    /// In Review, a task is its changes: they take the larger share. Any
    /// other status leads with the agent. A divider the person dragged wins,
    /// kept apart for the two kinds, and never past the edge.
    @Test("A task in review leads with its changes")
    func aTaskInReviewLeadsWithItsChanges() {
        #expect(TaskColumnModel.agentShare(status: .inReview, stored: nil) < 0.5)
        #expect(TaskColumnModel.agentShare(status: .inProgress, stored: nil) > 0.5)
        #expect(TaskColumnModel.agentShare(status: .needsDecision, stored: nil) > 0.5)
        #expect(TaskColumnModel.agentShare(status: .inReview, stored: 0.6) == 0.6)
        #expect(TaskColumnModel.agentShare(status: .inReview, stored: 0.99) == 1 - TaskColumnModel.minimumShare)
    }

    /// The task's own text leads: it's never collapsed, and with no agent
    /// and no changes to show, what's beneath it is one line, never a
    /// placeholder the height of the view (ov-79). The text's share is 40%
    /// until its divider is dragged, and never past the edge.
    @Test("With nothing working on a task, the space beneath its text is one line")
    func withNothingWorkingOnATaskTheSpaceBeneathItsTextIsOneLine() {
        let nothing = TaskColumnModel.work(.none(openWorktree: false), showsChanges: false)
        let noChanges = TaskColumnModel.work(.none(openWorktree: true), showsChanges: false)
        let changes = TaskColumnModel.work(.none(openWorktree: true), showsChanges: true)
        let live = TaskColumnModel.work(.live, showsChanges: false)
        #expect(nothing == .compact)
        #expect(noChanges == .compact)
        #expect(changes == .full)
        #expect(live == .full)
        #expect(TaskColumnModel.contentShare(stored: nil) == 0.4)
        #expect(TaskColumnModel.contentShare(stored: 0.7) == 0.7)
        #expect(TaskColumnModel.contentShare(stored: 0.01) == TaskColumnModel.minimumShare)
        #expect(TaskColumnModel.terminalCount(0) == "No terminals")
        #expect(TaskColumnModel.terminalCount(1) == "1 terminal")
        #expect(TaskColumnModel.terminalCount(2) == "2 terminals")
    }

    /// Drawn, not only decided: with no worktree and no agent, what's
    /// beneath a task's text is its one line, and the text has the rest of
    /// the height, Focus or not.
    @Test("With no worktree, the task's work is drawn as one line")
    func withNoWorktreeTheWorkIsDrawnAsOneLine() async {
        final class Seen { var content: CGFloat = 0 }
        struct Height: PreferenceKey {
            static let defaultValue: CGFloat = 0
            static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
        }
        for focused in [false, true] {
            let seen = Seen()
            let agent = TaskColumnModel.agent(hasAgent: false, worktree: nil)
            let view = TaskViewSplit(work: TaskColumnModel.work(agent, showsChanges: false), focused: focused) {
                GeometryReader { proxy in Color.clear.preference(key: Height.self, value: proxy.size.height) }
            } workArea: {
                TaskWorkHeader(
                    agent: agent, worktree: nil, agents: [], chosen: nil, onChooseAgent: { _ in },
                    onOpenWorktree: {})
            }
            .frame(width: 600, height: 500)
            .onPreferenceChange(Height.self) { value in MainActor.assumeIsolated { seen.content = value } }
            let host = NSHostingView(rootView: view)
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 600, height: 500), styleMask: [.borderless],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
            for _ in 0..<5 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
            window.close()
            // 500, less the 30 pt line and its 1 pt divider.
            #expect(seen.content == 469, "focused \(focused): the text got \(seen.content) of 500")
        }
    }

    /// Back goes up one level at a time: from a worktree opened from its
    /// task to the task, from the task to the workspace, and from there
    /// nowhere. Esc and ⌃⌘← put a popped-open orchestrator away first, then
    /// leave Focus; the breadcrumb's chevron does all three at once.
    @Test("Back goes worktree, task, workspace, after the orchestrator and Focus")
    func backGoesUpOneLevelAtATime() {
        let top = ContentView.Selection.workspace(host: "", workspace: "ws", focus: nil)
        let task = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .task("t-9"))
        let lane = WorkspaceNavigation.openWorktree("w-3", from: task)
        typealias Step = WorkspaceNavigation.BackStep
        func back(_ from: ContentView.Selection?, trail: ContentView.Selection?, peek: Bool = false, focus: Bool = false,
                  oneAtATime: Bool = true) -> Step {
            WorkspaceNavigation.backStep(peek: peek, focus: focus, oneAtATime: oneAtATime, from: from, trail: trail)
        }
        let fromLane = back(lane.next, trail: lane.trail)
        let fromTask = back(fromLane.goesTo, trail: nil)
        let fromTop = back(fromTask.goesTo, trail: nil)
        #expect(fromLane == Step(goesTo: task))
        #expect(fromTask == Step(goesTo: top))
        #expect(fromTop == Step(goesTo: nil))
        let peeked = back(task, trail: nil, peek: true, focus: true)
        let focused = back(task, trail: nil, focus: true)
        let chevron = back(task, trail: nil, peek: true, focus: true, oneAtATime: false)
        #expect(peeked == Step(closesPeek: true))
        #expect(focused == Step(leavesFocus: true))
        #expect(chevron == Step(closesPeek: true, leavesFocus: true, goesTo: top))
    }

    /// Where the keyboard lands after Back: the task's terminal, when the
    /// level it lands on shows one, else nothing, which is the view itself.
    @Test("After Back, the keyboard goes to the task's terminal, else the view")
    func afterBackTheKeyboardGoesToTheTasksTerminal() {
        let worktree = Worktree(
            id: "w-3", short: "w3", task: "fc-3", branch: "b", repository: nil, host: "", path: "/tmp/w3",
            state: "active", terminals: [])
        func rect(_ id: String, focused: Bool) -> PaneRect {
            PaneRect(id: id, short: id, title: nil, left: 0, top: 0, columns: 40, rows: 24, focused: focused, zoomed: false)
        }
        let group = PaneGroup(
            id: "@2", name: "", active: true, columns: 80, rows: 24, layout: "@2",
            panes: [rect("a1", focused: false), rect("a2", focused: true)])
        let agent = ShownLayout(column: .task, worktree: worktree, group: group, groups: [group])
        let task = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .task("t-9"))
        let keyed = WorkspaceScreen.keyPane(nil, in: [agent], selection: task)
        let none = WorkspaceScreen.keyPane(nil, in: [], selection: task)
        #expect(keyed == PaneRef(host: "", worktree: "w-3", terminal: "a2"))
        #expect(none == nil)
    }

    /// ⌥⌘2 from a task goes up to the workspace and leaves the keyboard on
    /// the board; any other change takes it back, as every navigation does.
    @Test("⌥⌘2 from a task keeps the keyboard on the board")
    func optionCommandTwoFromATaskKeepsTheKeyboardOnTheBoard() {
        let top = ContentView.Selection.workspace(host: "", workspace: "ws", focus: nil)
        let task = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .task("t-9"))
        let other = ContentView.Selection.workspace(host: "", workspace: "billing", focus: nil)
        let kept = WorkspaceNavigation.boardKeepsKeyboard(pending: true, from: task, to: top)
        let notAsked = WorkspaceNavigation.boardKeepsKeyboard(pending: false, from: task, to: top)
        let elsewhere = WorkspaceNavigation.boardKeepsKeyboard(pending: true, from: task, to: other)
        let fromTop = WorkspaceNavigation.boardKeepsKeyboard(pending: true, from: top, to: top)
        #expect(kept)
        #expect(!notAsked && !elsewhere && !fromTop)
    }

    /// The breadcrumb names each level down to the one you're at, and each
    /// above it goes back there: Workspace › Task, Workspace › Task ›
    /// Worktree for a worktree opened from its task, Workspace › Worktree
    /// for one opened from the sidebar. Nothing at the workspace's own level.
    @Test("The breadcrumb leads back up each level")
    func theBreadcrumbLeadsBackUpEachLevel() {
        let top = ContentView.Selection.workspace(host: "", workspace: "ws", focus: nil)
        let task = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .task("t-9"))
        let lane = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .worktree("w-3", terminal: "s"))
        func crumbs(_ selection: ContentView.Selection, trail: ContentView.Selection?) -> [WorkspaceNavigation.Crumb] {
            WorkspaceNavigation.crumbs(
                selection, trail: trail, workspace: "Main", task: { "bil-9 \($0)" }, worktree: { "wt \($0)" })
        }
        typealias Crumb = WorkspaceNavigation.Crumb
        let atTask = crumbs(task, trail: nil)
        let fromTask = crumbs(lane, trail: task)
        let fromSidebar = crumbs(lane, trail: nil)
        let atTop = crumbs(top, trail: nil)
        #expect(atTask == [Crumb(title: "Main", target: top), Crumb(title: "bil-9 t-9", target: nil)])
        #expect(
            fromTask == [
                Crumb(title: "Main", target: top), Crumb(title: "bil-9 t-9", target: task),
                Crumb(title: "wt w-3", target: nil),
            ])
        #expect(fromSidebar == [Crumb(title: "Main", target: top), Crumb(title: "wt w-3", target: nil)])
        #expect(atTop.isEmpty)
    }

    /// A task in review with no live agent can still reach its changes: its
    /// worktree, from `Task.worktree_id`.
    @Test("A task with no agent but a worktree offers Open Worktree")
    func aTaskWithNoAgentButAWorktreeOffersOpenWorktree() {
        let row = Self.row(.inReview, worktree: "w-3")
        let lane = TaskColumnModel.worktree(of: row, agent: nil)
        #expect(lane == "w-3")
        let agent = TaskColumnModel.agent(hasAgent: false, worktree: lane)
        #expect(agent == .none(openWorktree: true))
        #expect(TaskColumnModel.sentence(agent) == "No agent is working on this task.")
        #expect(TaskColumnModel.sentence(TaskColumnModel.agent(hasAgent: true, worktree: lane)) == nil)
    }

    @Test("A task with neither says nothing has started")
    func aTaskWithNeitherSaysNothingHasStarted() {
        let row = Self.row(.todo)
        let agent = TaskColumnModel.agent(hasAgent: false, worktree: TaskColumnModel.worktree(of: row, agent: nil))
        #expect(agent == .none(openWorktree: false))
        #expect(TaskColumnModel.sentence(agent) == "Nothing has started on this task yet.")
        // An empty id from the runner is no worktree either.
        #expect(TaskColumnModel.worktree(of: Self.row(.todo, worktree: ""), agent: nil) == nil)
    }

    /// A task's changes, drawn as the task view draws them
    /// (`TaskColumnChanges`), read the worktree's changes and split no pane,
    /// so nothing resizes the agent's window for other clients (ruling 6).
    /// The toolbar's Changes button in an opened worktree still splits one,
    /// and the recorder sees it, which is what shows it could see one.
    @Test("Opening a task's changes runs no split, and the toolbar's Changes button does")
    func openingATasksChangesRunsNoSplit() async {
        let worktree = Worktree(
            id: "w-3", short: "w3", task: "fc-3-webhooks", branch: "b", repository: "overnight", host: "",
            path: "/tmp/w3", state: "active", terminals: [])
        let column = WorktreeCallsTests.Recorder()
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in column.answer(args) }
        let store = ChangesStore(client: client, worktree: worktree)
        let host = NSHostingView(
            rootView: TaskColumnChanges(changes: store, isFocused: false, agents: []).frame(width: 500, height: 400))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 500, height: 400), styleMask: [.borderless], backing: .buffered,
            defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        for _ in 0..<100 where !column.calls.contains(where: { $0.first == "changes" }) {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(10))
        }
        window.close()
        #expect(column.calls.contains { $0.first == "changes" }, "the column never read its changes: \(column.calls)")
        #expect(!column.calls.contains { $0.starts(with: ["layout", "split"]) }, "\(column.calls)")

        let toolbar = WorktreeCallsTests.Recorder()
        let other = DaemonClient(target: "", notifications: NotificationCenter())
        other.commandRunnerForTesting = { args in toolbar.answer(args) }
        _ = await other.split(worktree, beside: nil, side: .right, preset: "changes", layout: nil)
        #expect(toolbar.calls.contains { $0.starts(with: ["layout", "split"]) && $0.contains("changes") }, "\(toolbar.calls)")
    }

    /// Open Worktree drills from a task into its worktree, whole, its own
    /// layouts in the bar, and Back returns to the task it came from. From
    /// its row under the workspace, with no task behind it, Back goes up to
    /// the workspace.
    @Test("Open Worktree shows the worktree's own layouts, and Back returns to the task")
    func openWorktreeShowsTheWorktreesOwnLayoutsAndBackReturnsToTheTask() {
        let task = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .task("t-9"))
        let opened = WorkspaceNavigation.openWorktree("w-3", from: task)
        #expect(opened.next == .workspace(host: "", workspace: "ws", focus: .worktree("w-3", terminal: nil)))
        #expect(opened.trail == task)

        var shell = Terminal(id: "s", short: "s", title: "zsh", preset: "zsh", state: "running", epoch: 0)
        shell.taskId = nil
        let lane = Worktree(
            id: "w-3", short: "w3", task: "fc-3-webhooks", branch: "b", repository: "overnight", host: "",
            path: "/tmp/w3", state: "active", terminals: [shell])
        func group(_ id: String, active: Bool) -> PaneGroup {
            PaneGroup(
                id: id, name: "", active: active, columns: 80, rows: 24, layout: id,
                panes: [PaneRect(id: "s", short: "s", title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: true, zoomed: false)])
        }
        let fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [lane], branchPrefix: nil)
        let shown = WorkspaceScreen.shown(opened.next, in: fleet, layouts: { _, _ in [group("@1", active: true), group("@2", active: false)] })
        #expect(shown.map(\.column) == [.worktree])
        #expect(shown.first?.groups.map(\.id) == ["@1", "@2"], "not the worktree's own layouts")

        #expect(WorkspaceNavigation.back(from: opened.next, trail: opened.trail) == task)
        #expect(WorkspaceNavigation.back(from: opened.next, trail: nil) == .workspace(host: "", workspace: "ws", focus: nil))
        #expect(WorkspaceNavigation.back(from: task, trail: nil) == .workspace(host: "", workspace: "ws", focus: nil))
        #expect(WorkspaceNavigation.back(from: .workspace(host: "", workspace: "ws", focus: nil), trail: task) == nil)
    }

    /// Esc goes Back only with something to go back from, and never while a
    /// terminal or a text field has the keyboard: a terminal needs its Esc,
    /// and a field cancels with it.
    @Test("Esc goes back only when no terminal or field has the keyboard")
    func escGoesBackOnlyWhenNoTerminalOrFieldHasTheKeyboard() {
        let task = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .task("t-9"))
        let plain = NSView()
        #expect(EscapeBack.goesBack(responder: plain, selection: task, focusColumn: false))
        #expect(EscapeBack.goesBack(responder: nil, selection: task, focusColumn: false))
        #expect(!EscapeBack.goesBack(responder: TerminalRenderView(), selection: task, focusColumn: false))
        #expect(!EscapeBack.goesBack(responder: NSTextView(), selection: task, focusColumn: false))
        #expect(!EscapeBack.goesBack(responder: NSTextField(), selection: task, focusColumn: false))
        // Nothing to go back from.
        let workspace = ContentView.Selection.workspace(host: "", workspace: "ws", focus: nil)
        #expect(!EscapeBack.goesBack(responder: plain, selection: workspace, focusColumn: false))
        #expect(!EscapeBack.goesBack(responder: plain, selection: .needsYou, focusColumn: false))
        #expect(EscapeBack.goesBack(responder: plain, selection: task, focusColumn: true))
    }

    /// The breadcrumb holds while the window is in the worktree Open
    /// Worktree opened, whichever of its panes is selected, and goes when
    /// anything else is chosen: another worktree from the disclosure
    /// included, which Back would otherwise have taken to the task.
    @Test("The breadcrumb goes when the opened worktree does")
    func theBreadcrumbGoesWhenTheOpenedWorktreeDoes() {
        let task = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .task("t-9"))
        let opened = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .worktree("w-3", terminal: nil))
        let pane = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .worktree("w-3", terminal: "s"))
        let other = ContentView.Selection.workspace(host: "", workspace: "ws", focus: .worktree("w-4", terminal: nil))
        #expect(WorkspaceNavigation.keeps(trail: task, opened: "w-3", now: opened))
        #expect(WorkspaceNavigation.keeps(trail: task, opened: "w-3", now: pane))
        #expect(!WorkspaceNavigation.keeps(trail: task, opened: "w-3", now: other))
        #expect(!WorkspaceNavigation.keeps(trail: task, opened: "w-3", now: task))
        #expect(!WorkspaceNavigation.keeps(trail: task, opened: nil, now: opened))
    }
}
