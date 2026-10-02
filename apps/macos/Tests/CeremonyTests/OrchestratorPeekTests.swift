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

/// The orchestrator popped open over a task from its rail (ov-84): a press
/// toggles it at once, mid-flight included, nothing waits on the motion, and
/// closed, it takes no click and isn't seen.
struct OrchestratorPeekTests {
    @MainActor
    private final class Level: ObservableObject {
        @Published var peek = false
        var made = 0
        var outOfSight: Bool?
        var enabled: Bool?
    }

    /// A workspace 1032 pt wide with a task open beside its board, beside a
    /// 200 pt sidebar, with the sidebar, the rail, the conversation, the
    /// board and what's opened each a view the hit test names, moving on
    /// `motion`. The rail is at x 200–228, the board's list at 229–529 and
    /// what's opened from 530; closed, the panel sits at about −207…205, so
    /// over the sidebar, where only a panel that leaked would take a click.
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
                        opened: "t" as String?, hasConversation: true, cell: WorkspaceColumns.defaultCell,
                        focused: false, peek: level.peek, listWidth: .constant(300),
                        conversation: {
                            MarkerView(
                                name: "conversation", onMake: { level.made += 1 },
                                onUpdate: { level.outOfSight = $0; level.enabled = $1 })
                        },
                        rail: { MarkerView(name: "rail") }, board: { MarkerView(name: "board") },
                        breadcrumb: { _ in Color.clear.frame(height: 30) }, detail: { _, _ in MarkerView(name: "opened") },
                        onDismissPeek: { level.peek = false }, motion: motion)
                    .frame(width: 1032, height: 400)
                }
            }
        }

        init(motion: Animation) {
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
        /// a marker's name, or "catcher" for SwiftUI's own content, which
        /// over what's opened is the click-outside catcher.
        func hit(_ x: CGFloat) -> String {
            let view = host.hitTest(NSPoint(x: x, y: 200))
            return (view as? Marker)?.name ?? "catcher"
        }
    }

    /// Slow enough that 40 ms in, the panel has barely moved.
    private static let slow = Animation.spring(response: 3, dampingFraction: 1)

    /// The rail is where the owner clicks again: it must take the click
    /// while the panel slides either way. Before ov-84 the panel slid by
    /// its whole width, catcher and all, across the rail, and stayed
    /// hit-testable while removed, so a click there mid-close hit the
    /// catcher and did nothing.
    @MainActor
    @Test("The rail takes a click while the panel moves, both ways")
    func theRailTakesAClickWhileThePanelMoves() async {
        let harness = Harness(motion: Self.slow)
        await harness.settle()
        #expect(harness.hit(210) == "rail")
        #expect(harness.hit(100) == "sidebar")
        harness.level.peek = true
        await harness.settle(2)
        #expect(harness.hit(210) == "rail", "opening")
        // Partway, with the panel's leading edge still under the rail.
        try? await Task.sleep(for: .milliseconds(500))
        await harness.settle(1)
        #expect(harness.hit(210) == "rail", "partway open")
        // The panel's far side is over the sidebar here: only the clip keeps
        // it from taking the click.
        #expect(harness.hit(100) == "sidebar", "partway open")
        try? await Task.sleep(for: .seconds(3.5))
        await harness.settle()
        #expect(harness.hit(210) == "rail", "open")
        #expect(harness.hit(329) == "conversation", "open")
        harness.level.peek = false
        await harness.settle(2)
        #expect(harness.hit(210) == "rail", "closing")
        // Closed, though still sliding: the click goes to what's under it.
        #expect(harness.hit(329) == "board", "closing")
        #expect(harness.hit(1100) == "opened", "closing")
        try? await Task.sleep(for: .milliseconds(500))
        await harness.settle(1)
        #expect(harness.hit(210) == "rail", "partway closed")
        #expect(harness.hit(100) == "sidebar", "partway closed")
        #expect(harness.hit(329) == "board", "partway closed")
        harness.window.close()
    }

    /// Five presses inside a tenth of a second: each one counts, the last
    /// decides, and what takes a click follows the state at once rather
    /// than the motion. The terminal view is made once, on the first open,
    /// not per open.
    @MainActor
    @Test("Rapid presses end in the state last pressed, on one terminal view")
    func rapidPressesEndInTheStateLastPressed() async {
        let harness = Harness(motion: Self.slow)
        await harness.settle()
        var open = false
        for _ in 0..<5 {
            open = OrchestratorPeek.pressed(open: open)
            harness.level.peek = open
            await harness.settle(1)
        }
        #expect(open)
        // Open: a click over what's opened is the panel's or the catcher's.
        #expect(harness.hit(1100) == "catcher")
        open = OrchestratorPeek.pressed(open: open)
        harness.level.peek = open
        await harness.settle(1)
        #expect(!open)
        #expect(harness.hit(1100) == "opened")
        #expect(harness.hit(329) == "board")
        open = OrchestratorPeek.pressed(open: open)
        harness.level.peek = open
        await harness.settle(1)
        #expect(harness.hit(1100) == "catcher")
        #expect(harness.level.made == 1, "the conversation was rebuilt \(harness.level.made) times")
        harness.window.close()
    }

    /// Closed and settled, the panel is off to the side: its view is still
    /// there, for the next open, but nothing of it is drawn or clicked.
    @MainActor
    @Test("A closed panel isn't clicked or drawn")
    func aClosedPanelIsntClickedOrDrawn() async {
        let harness = Harness(motion: .linear(duration: 0.05))
        await harness.settle()
        harness.level.peek = true
        await harness.settle(10)
        #expect(harness.hit(329) == "conversation")
        harness.level.peek = false
        await harness.settle(10)
        #expect(harness.hit(329) == "board")
        #expect(harness.hit(210) == "rail")
        // Where the closed panel sits: the sidebar's, not the panel's.
        #expect(harness.hit(100) == "sidebar")
        #expect(harness.hit(190) == "sidebar")
        // Opened again, it's the same view, slid back out.
        harness.level.peek = true
        await harness.settle(10)
        #expect(harness.hit(329) == "conversation")
        #expect(harness.level.made == 1, "the conversation was rebuilt \(harness.level.made) times")
        harness.window.close()
        // And as values: off past its own width and shadow, and no clicks.
        let width = WorkspaceColumns.peekWidth(in: 1032)
        #expect(OrchestratorPeek.offset(open: false, width: width) < -width)
        #expect(OrchestratorPeek.offset(open: true, width: width) == 0)
        #expect(!OrchestratorPeek.takesClicks(open: false))
        #expect(OrchestratorPeek.takesClicks(open: true))
    }

    /// Closed, nothing in the panel takes the keyboard: what's inside reads
    /// `outOfSight` and disabled, at once. Open, both come back. (Hidden
    /// from VoiceOver is checked live, through the AX API, in the ov-84
    /// report: SwiftUI builds no accessibility tree in this test host.)
    @MainActor
    @Test("A closed panel is out of the keyboard's reach")
    func aClosedPanelIsOutOfTheKeyboardsReach() async {
        let harness = Harness(motion: .linear(duration: 0.05))
        await harness.settle()
        harness.level.peek = true
        await harness.settle(10)
        #expect(harness.level.outOfSight == false)
        #expect(harness.level.enabled == true)
        harness.level.peek = false
        await harness.settle(1)
        // At once, not when the motion ends.
        #expect(harness.level.outOfSight == true)
        #expect(harness.level.enabled == false)
        await harness.settle(10)
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

    /// The panel draws the conversation's layout closed as well as open, so
    /// it's one terminal view; but it's on screen (seen, watched) and has the
    /// keyboard only while popped open, over the board alone or a task
    /// beside it.
    @Test("A closed panel draws the conversation without seeing, watching or typing into it")
    func aClosedPanelDrawsWithoutSeeingOrTyping() {
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
        let cases: [(WorkspaceColumns.Arrangement, Bool)] = [
            (.beside, false), (.besidePeeked, true), (.workspace, false), (.peeked, true), (.alone, false),
        ]
        for (arrangement, open) in cases {
            let visible = WorkspaceScreen.visible(all, arrangement: arrangement)
            let drawn = OrchestratorPeek.conversation(visible: visible, drawable: all)
            #expect(drawn.layout?.group.id == "@1", "drawn either way")
            #expect(drawn.onScreen == open)
            // What's seen and watched (`visibleTerminals`) comes from `visible`.
            #expect(visible.contains { $0.column == .conversation } == open)
            #expect(
                OrchestratorPeek.takesKeyboard(drawn.layout!, onScreen: drawn.onScreen, key: key, onBoard: false) == open)
        }
    }
}
