import AppKit
import AgentKit
import Foundation
import SwiftUI
import Testing

@testable import Far_Cooler

/// A task's three tabs, Overview, Agent and Changes, under its header
/// (ov-98).
@MainActor
struct TaskTabsTests {
    /// A task opens on its agent while one is working on it, else on its
    /// overview; once a tab is chosen for a task it's the one that task
    /// opens on, and no other task's.
    @Test("A task opens on Agent while one works it, else Overview, and remembers the tab chosen")
    func theDefaultTabAndItsMemory() {
        var memory = TaskTabMemory()
        #expect(memory.tab(for: "t-1", agentWorking: true) == .agent)
        #expect(memory.tab(for: "t-1", agentWorking: false) == .overview)
        memory.choose(.changes, for: "t-1")
        #expect(memory.tab(for: "t-1", agentWorking: true) == .changes)
        #expect(memory.tab(for: "t-1", agentWorking: false) == .changes)
        #expect(memory.tab(for: "t-2", agentWorking: true) == .agent, "another task's choice leaked")
        memory.choose(.overview, for: "t-2")
        #expect(memory.tab(for: "t-2", agentWorking: true) == .overview)
        #expect(memory.tab(for: "t-1", agentWorking: true) == .changes)
    }

    /// ⌃⌘] and ⌃⌘[ step through the tabs in order, wrapping, from the one
    /// shown, and the step is remembered as a choice.
    @Test("⌃⌘] and ⌃⌘[ walk the tabs, wrapping, from the one shown")
    func theShortcutsWalkTheTabs() {
        var memory = TaskTabMemory()
        #expect(TaskTab.allCases == [.overview, .agent, .changes])
        memory.step("t-1", by: 1, agentWorking: false)
        #expect(memory.tab(for: "t-1", agentWorking: false) == .agent)
        memory.step("t-1", by: 1, agentWorking: false)
        #expect(memory.tab(for: "t-1", agentWorking: false) == .changes)
        memory.step("t-1", by: 1, agentWorking: false)
        #expect(memory.tab(for: "t-1", agentWorking: false) == .overview, "doesn't wrap forward")
        memory.step("t-1", by: -1, agentWorking: false)
        #expect(memory.tab(for: "t-1", agentWorking: false) == .changes, "doesn't wrap back")
        // From the default: a task an agent is working opens on Agent, so
        // ⌃⌘[ goes to Overview.
        memory.step("t-2", by: -1, agentWorking: true)
        #expect(memory.tab(for: "t-2", agentWorking: true) == .overview)
        #expect(TaskTab.overview.title == "Overview" && TaskTab.agent.title == "Agent" && TaskTab.changes.title == "Changes")
    }

    /// The task's terminal counts as on screen, for seen marks, the
    /// watching claim and the keyboard, only while its Agent tab is shown.
    /// With Changes or Overview in front it's hidden: nothing is watched
    /// for it. The orchestrator's own rule is untouched.
    @Test("With Changes shown, the task's terminal isn't watched")
    func watchingFollowsTheTab() {
        let worktree = Worktree(
            id: "w", short: "w", task: "w", branch: "b", repository: nil, host: "", path: "/tmp/w",
            state: "active", terminals: [])
        func layout(_ column: ShownLayout.Column, _ id: String, _ pane: String) -> ShownLayout {
            let rect = PaneRect(
                id: pane, short: pane, title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: true, zoomed: false)
            let group = PaneGroup(id: id, name: "", active: true, columns: 80, rows: 24, layout: id, panes: [rect])
            return ShownLayout(column: column, worktree: worktree, group: group, groups: [group])
        }
        let all = [layout(.conversation, "@1", "conductor"), layout(.task, "@2", "agent")]
        func columns(_ tab: TaskTab, _ arrangement: WorkspaceColumns.Arrangement) -> [ShownLayout.Column] {
            WorkspaceScreen.visible(all, arrangement: arrangement, taskTab: tab).map(\.column)
        }
        #expect(columns(.agent, .opened) == [.task])
        #expect(columns(.changes, .opened) == [], "the agent is watched behind the Changes tab")
        #expect(columns(.overview, .opened) == [])
        #expect(columns(.changes, .alone) == [])
        #expect(columns(.agent, .alone) == [.task])
        #expect(columns(.changes, .workspace) == [.conversation])
        // An opened worktree has no tabs: its layout is on screen whatever
        // a task last showed.
        let lane = [layout(.worktree, "@3", "shell")]
        #expect(WorkspaceScreen.visible(lane, arrangement: .opened, taskTab: .changes).map(\.column) == [.worktree])
    }

    /// The agent's terminal is made once and kept while the tabs switch
    /// around it: hidden, never rebuilt, so it never re-wraps. Hidden, it's
    /// out of sight, so it takes no keyboard.
    @Test("Switching tabs keeps the one terminal view")
    func switchingTabsKeepsTheTerminal() async {
        final class Probe {
            var made: [NSView] = []
            var outOfSight: [Bool] = []
        }
        struct Terminal: NSViewRepresentable {
            let probe: Probe
            func makeNSView(context: Context) -> NSView {
                let view = NSView()
                probe.made.append(view)
                return view
            }
            func updateNSView(_ nsView: NSView, context: Context) {
                probe.outOfSight.append(context.environment.outOfSight)
            }
        }
        final class Shown: ObservableObject { @Published var tab: TaskTab = .agent }
        struct Host: View {
            @ObservedObject var shown: Shown
            let probe: Probe
            var body: some View {
                TaskTabs(tab: shown.tab) {
                    Text("Overview")
                } agent: {
                    Terminal(probe: probe)
                } changes: {
                    Text("Changes")
                }
                .frame(width: 400, height: 300)
            }
        }
        let probe = Probe()
        let shown = Shown()
        let host = NSHostingView(rootView: Host(shown: shown, probe: probe))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 400, height: 300), styleMask: [.borderless], backing: .buffered,
            defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        func settle() async {
            for _ in 0..<5 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
        await settle()
        let first = probe.made.first
        #expect(probe.outOfSight.last == false)
        for tab in [TaskTab.changes, .overview, .agent, .overview, .changes, .agent] {
            shown.tab = tab
            await settle()
            #expect(probe.outOfSight.last == (tab != .agent), "\(tab): out of sight \(String(describing: probe.outOfSight.last))")
        }
        window.close()
        #expect(probe.made.count == 1, "the terminal was made \(probe.made.count) times")
        #expect(probe.made.first === first)
    }

    /// Prose is set to a reading measure of about seventy characters of
    /// the body's own type, however wide the window.
    @Test("The overview's prose is about seventy characters wide")
    func theMeasureIsAboutSeventyCharacters() {
        let sample = "The ticket view renders Markdown, with better type and three tabs to read it in. "
        let font = NSFont.preferredFont(forTextStyle: .body)
        let width = (sample as NSString).size(withAttributes: [.font: font]).width
        let perCharacter = width / CGFloat(sample.count)
        let characters = TaskTypography.measure / perCharacter
        #expect((65...75).contains(characters), "\(characters) characters at \(TaskTypography.measure) pt")
    }

    /// A task leaving the main area is out of sight as a whole: its front
    /// tab included (review M1). The tabs' own `outOfSight` adds to the
    /// pane's rather than replacing it, so a terminal on the Agent tab of a
    /// task switched away from lets go of the keyboard.
    @Test("The front tab of a leaving task is out of sight too")
    func theFrontTabOfALeavingTaskIsOutOfSight() async {
        final class Seen { var outOfSight: [Bool] = [] }
        struct Probe: NSViewRepresentable {
            let seen: Seen
            func makeNSView(context: Context) -> NSView { NSView() }
            func updateNSView(_ nsView: NSView, context: Context) {
                seen.outOfSight.append(context.environment.outOfSight)
            }
        }
        for leaving in [false, true] {
            let seen = Seen()
            let view = TaskTabs(tab: .agent) {
                Text("Overview")
            } agent: {
                Probe(seen: seen)
            } changes: {
                Text("Changes")
            }
            .environment(\.outOfSight, leaving)
            .frame(width: 300, height: 200)
            let host = NSHostingView(rootView: view)
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 300, height: 200), styleMask: [.borderless],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
            for _ in 0..<5 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
            window.close()
            #expect(seen.outOfSight.last == leaving, "leaving \(leaving): the agent tab saw \(seen.outOfSight)")
        }
    }

    /// What the Agent tab draws: the start panel with no agent, nothing
    /// while a glance passes over the task, the agent's layout once read,
    /// else its terminal alone. The same whichever tab is in front, so
    /// switching tabs never swaps one view for another (no re-wrap).
    @Test("The Agent tab's view doesn't depend on which tab is in front")
    func theAgentTabsView() {
        typealias Model = TaskColumnModel
        #expect(Model.agentView(hasAgent: false, settled: true, hasLayout: false) == .start)
        #expect(Model.agentView(hasAgent: false, settled: false, hasLayout: true) == .start)
        #expect(Model.agentView(hasAgent: true, settled: false, hasLayout: true) == .waiting)
        #expect(Model.agentView(hasAgent: true, settled: true, hasLayout: true) == .tiled)
        #expect(Model.agentView(hasAgent: true, settled: true, hasLayout: false) == .bare)
    }

    /// The diff's keys (⌥⌘↓, ⌥⌘], ⌃⌥⌘], Mark as Reviewed) are for a diff
    /// you can see: only while Changes is in front (review minor 2).
    @Test("A diff behind another tab doesn't take the Diff menu's keys")
    func aHiddenDiffTakesNoKeys() {
        #expect(TaskColumnModel.changesFocused(focus: "t-1", task: "t-1", tab: .changes))
        #expect(!TaskColumnModel.changesFocused(focus: "t-1", task: "t-1", tab: .overview))
        #expect(!TaskColumnModel.changesFocused(focus: "t-1", task: "t-1", tab: .agent))
        #expect(!TaskColumnModel.changesFocused(focus: "t-2", task: "t-1", tab: .changes))
        #expect(!TaskColumnModel.changesFocused(focus: nil, task: "t-1", tab: .changes))
    }

    /// ⌃⌘] and ⌃⌘[ are enabled only with a task open (review minor 3).
    @Test("The tab shortcuts are enabled only with a task open")
    func tabShortcutsNeedATask() {
        #expect(MainWindowFocus.stepsTaskTabs(MainWindowFocus(overlayOpen: false, taskOpen: true)))
        #expect(!MainWindowFocus.stepsTaskTabs(MainWindowFocus(overlayOpen: false, taskOpen: false)))
        #expect(!MainWindowFocus.stepsTaskTabs(MainWindowFocus(overlayOpen: true, taskOpen: true)))
        #expect(!MainWindowFocus.stepsTaskTabs(nil))
    }
}
