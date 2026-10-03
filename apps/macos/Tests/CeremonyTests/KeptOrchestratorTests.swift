import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A view the hosting view's own hit test can name.
private final class Marker: NSView {
    var name = ""
}

private struct MarkerView: NSViewRepresentable {
    let name: String
    var onMake: () -> Void = {}
    /// What the environment says about this view: out of sight, enabled.
    var onUpdate: (_ outOfSight: Bool, _ enabled: Bool) -> Void = { _, _ in }
    func makeNSView(context: Context) -> Marker {
        onMake()
        let view = Marker()
        view.name = name
        onUpdate(context.environment.outOfSight, context.environment.isEnabled)
        return view
    }
    func updateNSView(_ view: Marker, context: Context) {
        onUpdate(context.environment.outOfSight, context.environment.isEnabled)
    }
}

/// The orchestrator kept mounted while a task or a worktree is selected
/// (ov-92): the main area follows the selection at once, mid-flight
/// included, nothing waits on the motion, its terminal view is made once,
/// and hidden, it takes no click, no keyboard and isn't seen.
struct KeptOrchestratorTests {
    @MainActor
    private final class Level: ObservableObject {
        @Published var opened: String?
        @Published var focused = false
        var made = 0
        var outOfSight: Bool?
        var enabled: Bool?
    }

    /// A workspace 1032 pt wide beside a 200 pt sidebar, with the sidebar,
    /// the navigator, the conversation and what's opened each a view the
    /// hit test names, moving on `motion`. The navigator is at x 200–480
    /// and the main area from 481 (ov-92).
    @MainActor
    private final class Harness {
        let level = Level()
        let host: NSHostingView<Hosted>
        let window: NSWindow

        struct Hosted: View {
            @ObservedObject var level: Level
            let motion: Animation
            var body: some View {
                HStack(spacing: 0) {
                    MarkerView(name: "sidebar").frame(width: 200)
                    WorkspaceView(
                        opened: level.opened, hasConversation: true, cell: WorkspaceColumns.defaultCell,
                        focused: level.focused, navigatorWidth: .constant(280),
                        conversation: {
                            MarkerView(
                                name: "conversation", onMake: { level.made += 1 },
                                onUpdate: { level.outOfSight = $0; level.enabled = $1 })
                        },
                        navigator: { MarkerView(name: "navigator") },
                        breadcrumb: { _ in Color.clear.frame(height: ColumnHeader.total) },
                        detail: { _, _ in MarkerView(name: "opened") }, motion: motion)
                    .frame(width: 1032, height: 400)
                }
            }
        }

        init(motion: Animation, opened: String? = nil) {
            level.opened = opened
            host = NSHostingView(rootView: Hosted(level: level, motion: motion))
            window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 1232, height: 400), styleMask: [.borderless],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = host
        }

        func settle(_ frames: Int = 5) async {
            for _ in 0..<frames {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }

        /// What a click `x` pt in from the window's leading edge lands on:
        /// a marker's name, or "none" for SwiftUI's own content.
        func hit(_ x: CGFloat) -> String {
            let view = host.hitTest(NSPoint(x: x, y: 200))
            return (view as? Marker)?.name ?? "none"
        }
    }

    /// Slow enough that 40 ms in, the cross-fade has barely begun.
    private static let slow = Animation.spring(response: 3, dampingFraction: 1)

    /// Selecting a task and the orchestrator again, five times inside a
    /// tenth of a second on a slow spring: what takes a click follows the
    /// selection at once, never the fade, the navigator takes its own
    /// throughout, and the orchestrator's view is made once.
    @MainActor
    @Test("The main area follows the selection at once, on one terminal view")
    func theMainAreaFollowsTheSelectionAtOnce() async {
        let harness = Harness(motion: Self.slow)
        await harness.settle()
        #expect(harness.hit(100) == "sidebar")
        #expect(harness.hit(300) == "navigator")
        #expect(harness.hit(800) == "conversation")
        for index in 0..<5 {
            harness.level.opened = index.isMultiple(of: 2) ? "t" : nil
            await harness.settle(1)
            let expected = index.isMultiple(of: 2) ? "opened" : "conversation"
            #expect(harness.hit(800) == expected, "step \(index), mid-fade")
            #expect(harness.hit(300) == "navigator", "step \(index)")
        }
        // Left on the task: the orchestrator is hidden, not gone.
        try? await Task.sleep(for: .milliseconds(500))
        await harness.settle(1)
        #expect(harness.hit(800) == "opened")
        harness.level.opened = nil
        await harness.settle(1)
        #expect(harness.hit(800) == "conversation")
        #expect(harness.level.made == 1, "the conversation was rebuilt \(harness.level.made) times")
        harness.window.close()
    }

    /// Focus puts the navigator away to the left, past the sidebar, where
    /// it takes no click, and what's opened takes the whole detail.
    @MainActor
    @Test("Focus puts the navigator away and gives the main area the detail")
    func focusPutsTheNavigatorAway() async {
        let harness = Harness(motion: .linear(duration: 0.05))
        await harness.settle()
        harness.level.opened = "t"
        await harness.settle(10)
        #expect(harness.hit(600) == "opened", "before focus")
        harness.level.focused = true
        await harness.settle(10)
        #expect(harness.hit(300) == "opened")
        #expect(harness.hit(210) == "opened")
        #expect(harness.hit(100) == "sidebar")
        harness.level.focused = false
        await harness.settle(10)
        #expect(harness.hit(300) == "navigator")
        harness.window.close()
    }

    /// A window reopening on a task draws it from the first frame, the
    /// orchestrator mounted and hidden behind it.
    @MainActor
    @Test("Reopened on a task, it's drawn and takes the click from the first frame")
    func reopenedOnATask() async {
        let harness = Harness(motion: .linear(duration: 0.05), opened: "t")
        await harness.settle()
        #expect(harness.hit(600) == "opened")
        #expect(harness.hit(300) == "navigator")
        #expect(harness.level.made == 1)
        #expect(harness.level.outOfSight == true)
        harness.window.close()
    }

    /// Hidden, nothing in the orchestrator takes the keyboard: what's inside
    /// reads `outOfSight` and disabled, at once. Selected, both come back.
    /// (Hidden from VoiceOver is checked live, through the AX API: SwiftUI
    /// builds no accessibility tree in this test host.)
    @MainActor
    @Test("A hidden orchestrator is out of the keyboard's reach")
    func aHiddenOrchestratorIsOutOfTheKeyboardsReach() async {
        let harness = Harness(motion: .linear(duration: 0.05))
        await harness.settle(10)
        #expect(harness.level.outOfSight == false)
        #expect(harness.level.enabled == true)
        harness.level.opened = "t"
        await harness.settle(1)
        // At once, not when the motion ends.
        #expect(harness.level.outOfSight == true)
        #expect(harness.level.enabled == false)
        harness.level.opened = nil
        await harness.settle(1)
        #expect(harness.level.outOfSight == false)
        #expect(harness.level.enabled == true)
        harness.window.close()
    }

    /// A terminal out of sight refuses the keyboard, from a click, Tab or a
    /// claim, and one that held it lets go, so it never types into a panel
    /// nobody can see.
    @MainActor
    @Test("A terminal out of sight refuses and lets go of the keyboard")
    func aTerminalOutOfSightLetsGoOfTheKeyboard() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 400, height: 300), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let terminal = TerminalRenderView()
        terminal.frame = NSRect(x: 0, y: 0, width: 200, height: 300)
        let other = NSTextField(frame: NSRect(x: 220, y: 10, width: 150, height: 24))
        window.contentView?.addSubview(terminal)
        window.contentView?.addSubview(other)
        #expect(window.makeFirstResponder(terminal))
        terminal.takesKeyboard = false
        #expect(window.firstResponder !== terminal, "it kept the keyboard out of sight")
        #expect(!terminal.acceptsFirstResponder)
        #expect(!terminal.canBecomeKeyView, "Tab can still reach it")
        window.makeFirstResponder(terminal)
        #expect(window.firstResponder !== terminal, "a claim took the keyboard out of sight")
        terminal.takesKeyboard = true
        #expect(window.makeFirstResponder(terminal))
        window.close()
    }

    /// The orchestrator's view draws its layout hidden as well as selected,
    /// so it's one terminal view; but it's on screen (seen, watched) and has
    /// the keyboard only while it's selected (ov-92).
    @Test("A hidden orchestrator draws its layout without seeing, watching or typing into it")
    func aHiddenOrchestratorDrawsWithoutSeeingOrTyping() {
        let worktree = Worktree(
            id: "w", short: "w", task: "w", branch: "b", repository: nil, host: "", path: "/tmp/w",
            state: "active", terminals: [])
        func layout(_ column: ShownLayout.Column, _ id: String, _ pane: String) -> ShownLayout {
            let rect = PaneRect(id: pane, short: pane, title: nil, left: 0, top: 0, columns: 80, rows: 24, focused: true, zoomed: false)
            let group = PaneGroup(id: id, name: "", active: true, columns: 80, rows: 24, layout: id, panes: [rect])
            return ShownLayout(column: column, worktree: worktree, group: group, groups: [group])
        }
        let all = [layout(.conversation, "@1", "conductor"), layout(.task, "@2", "agent")]
        let key = PaneRef(host: "", worktree: "w", terminal: "conductor")
        let cases: [(WorkspaceColumns.Arrangement, Bool)] = [(.workspace, true), (.opened, false), (.alone, false)]
        for (arrangement, open) in cases {
            let visible = WorkspaceScreen.visible(all, arrangement: arrangement)
            let drawn = KeptOrchestrator.conversation(visible: visible, drawable: all)
            #expect(drawn.layout?.group.id == "@1", "drawn either way")
            #expect(drawn.onScreen == open)
            // What's seen and watched (`visibleTerminals`) comes from `visible`.
            #expect(visible.contains { $0.column == .conversation } == open)
            #expect(
                KeptOrchestrator.takesKeyboard(drawn.layout!, onScreen: drawn.onScreen, key: key, onBoard: false) == open)
        }
    }
}
