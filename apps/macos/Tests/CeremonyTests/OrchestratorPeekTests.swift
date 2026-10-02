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
    func makeNSView(context: Context) -> Marker {
        onMake()
        let view = Marker()
        view.name = name
        return view
    }
    func updateNSView(_ view: Marker, context: Context) {}
}

/// The orchestrator popped open over a task from its rail (ov-84): a press
/// toggles it at once, mid-flight included, nothing waits on the motion, and
/// closed, it takes no click and isn't seen.
struct OrchestratorPeekTests {
    @MainActor
    private final class Level: ObservableObject {
        @Published var peek = false
        var made = 0
    }

    /// A drilled-in workspace 1032 pt wide, with the rail, the conversation
    /// and what's opened each a view the hit test names, moving on `motion`.
    @MainActor
    private final class Harness {
        let level = Level()
        let host: NSHostingView<Hosted>
        let window: NSWindow

        struct Hosted: View {
            @ObservedObject var level: Level
            let motion: Animation
            var body: some View {
                WorkspaceView(
                    drilled: true, hasConversation: true, cell: WorkspaceColumns.defaultCell,
                    focused: false, peek: level.peek, pick: .constant(.orchestrator),
                    conversation: { MarkerView(name: "conversation", onMake: { level.made += 1 }) },
                    rail: { MarkerView(name: "rail") }, board: { Color.clear },
                    breadcrumb: { Color.clear.frame(height: 30) }, opened: { MarkerView(name: "opened") },
                    onDismissPeek: { level.peek = false }, peekMotion: motion)
                .frame(width: 1032, height: 400)
            }
        }

        init(motion: Animation) {
            host = NSHostingView(rootView: Hosted(level: level, motion: motion))
            window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 1032, height: 400), styleMask: [.borderless],
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

        /// What a click `x` pt in from the detail's leading edge lands on:
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
        #expect(harness.hit(10) == "rail")
        harness.level.peek = true
        await harness.settle(2)
        #expect(harness.hit(10) == "rail", "opening")
        // Partway, with the panel's leading edge still under the rail.
        try? await Task.sleep(for: .milliseconds(500))
        await harness.settle(1)
        #expect(harness.hit(10) == "rail", "partway open")
        try? await Task.sleep(for: .seconds(3.5))
        await harness.settle()
        #expect(harness.hit(10) == "rail", "open")
        #expect(harness.hit(100) == "conversation", "open")
        harness.level.peek = false
        await harness.settle(2)
        #expect(harness.hit(10) == "rail", "closing")
        // Closed, though still sliding: the click goes to what's opened.
        #expect(harness.hit(100) == "opened", "closing")
        #expect(harness.hit(900) == "opened", "closing")
        try? await Task.sleep(for: .milliseconds(500))
        await harness.settle(1)
        #expect(harness.hit(10) == "rail", "partway closed")
        #expect(harness.hit(100) == "opened", "partway closed")
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
        #expect(harness.hit(900) == "catcher")
        open = OrchestratorPeek.pressed(open: open)
        harness.level.peek = open
        await harness.settle(1)
        #expect(!open)
        #expect(harness.hit(900) == "opened")
        #expect(harness.hit(100) == "opened")
        open = OrchestratorPeek.pressed(open: open)
        harness.level.peek = open
        await harness.settle(1)
        #expect(harness.hit(900) == "catcher")
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
        #expect(harness.hit(100) == "conversation")
        harness.level.peek = false
        await harness.settle(10)
        #expect(harness.hit(100) == "opened")
        #expect(harness.hit(10) == "rail")
        // Opened again, it's the same view, slid back out.
        harness.level.peek = true
        await harness.settle(10)
        #expect(harness.hit(100) == "conversation")
        #expect(harness.level.made == 1, "the conversation was rebuilt \(harness.level.made) times")
        harness.window.close()
        // And as values: off past its own width and shadow, and no clicks.
        let width = WorkspaceColumns.peekWidth(in: 1032)
        #expect(OrchestratorPeek.offset(open: false, width: width) < -width)
        #expect(OrchestratorPeek.offset(open: true, width: width) == 0)
        #expect(!OrchestratorPeek.takesClicks(open: false))
        #expect(OrchestratorPeek.takesClicks(open: true))
    }

    /// The toggle is a function of the state alone: any number of presses,
    /// at any pace, end open after an odd number and closed after an even.
    @Test("Every press toggles")
    func everyPressToggles() {
        var open = false
        for press in 1...9 {
            open = OrchestratorPeek.pressed(open: open)
            #expect(open == (press % 2 == 1))
        }
    }
}
