import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The title bar's workspace switcher is as wide as what it says, however
/// late that arrives (ov-177, round 2).
///
/// A window's switcher reads "Workspaces" until it has picked a workspace.
/// When that lands after the window's content is installed but before it's
/// on screen, as in a window opened with the store already loaded, the
/// toolbar kept the item at the width it measured for "Workspaces", 102 pt,
/// and clipped "Main · overnight" (114 pt) on both sides. A real titled window,
/// because only the window's toolbar measures items: the offscreen renderer
/// `GridGeometryTests` uses has no title bar.
@MainActor
@Suite(.serialized)
struct SwitcherWidthTests {
    /// What the switcher says, changed from outside as the store does.
    @MainActor @Observable final class Label {
        var title = "Workspaces"
        var repository = ""
    }

    /// The window's leading toolbar as `ContentView` wires it: on the
    /// window's root view, with no `NavigationSplitView` since the old
    /// Fleet sidebar went (ov-178).
    struct Window: View {
        let label: Label
        var body: some View {
            WorkspaceStyle.canvas
                .toolbar(removing: .title)
                .toolbar {
                    LeadingToolbar(
                        switcher: WorkspaceSwitcherButton(
                            title: label.title, repository: label.repository, entries: [], openRequest: 0,
                            perform: { _ in }),
                        navigator: NavigatorToggle(hidden: false, available: true, toggle: {}))
                }
        }
    }

    /// The switcher's widths: what its label takes, and what the toolbar
    /// gave its item. The label's are its menu anchor's, which sits behind
    /// the whole label.
    struct Widths: Equatable {
        let label: CGFloat
        let item: CGFloat
    }

    /// When the label changes from "Workspaces" to "Main · overnight".
    enum When: CustomStringConvertible {
        /// Before the window's content is installed: no change to see.
        case fromTheStart
        /// After its toolbar has measured the item, before it shows: the
        /// launch race.
        case beforeShowing
        /// Once it's on screen.
        case onScreen

        var description: String {
            switch self {
            case .fromTheStart: "from the start"
            case .beforeShowing: "before showing"
            case .onScreen: "on screen"
            }
        }
    }

    /// The switcher's widths in a window whose label changes `when`.
    private static func widths(_ when: When) async throws -> Widths? {
        let label = Label()
        if when == .fromTheStart { loaded(label) }
        // Off every screen, so a run draws nothing on the owner's.
        let window = NSWindow(
            contentRect: NSRect(x: -6000, y: -6000, width: 1000, height: 200),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.toolbarStyle = .unified
        defer { window.close() }
        window.contentViewController = NSHostingController(rootView: Window(label: label))
        window.setContentSize(NSSize(width: 1000, height: 200))
        if when == .beforeShowing { loaded(label) }
        window.orderFrontRegardless()
        try await settle(window)
        if when == .onScreen {
            loaded(label)
            try await settle(window)
        }
        return measure(window.contentView?.superview)
    }

    private static func settle(_ window: NSWindow) async throws {
        for _ in 0..<15 {
            window.contentView?.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(40))
        }
    }

    private static func measure(_ view: NSView?) -> Widths? {
        guard let view else { return nil }
        if String(describing: type(of: view)) == "Anchor" {
            var item: NSView? = view.superview
            while let current = item, !String(describing: type(of: current)).hasPrefix("ToolbarItemHostingView") {
                item = current.superview
            }
            guard let item else { return nil }
            return Widths(label: view.frame.width, item: item.frame.width)
        }
        for sub in view.subviews { if let found = measure(sub) { return found } }
        return nil
    }

    private static func loaded(_ label: Label) {
        label.title = "Main"
        label.repository = "overnight"
    }

    @Test("The switcher is as wide as its label, whenever the label arrives", arguments: [When.beforeShowing, .onScreen])
    func measuredAgain(when: When) async throws {
        let fresh = try #require(try await Self.widths(.fromTheStart))
        // Room for the whole label, and the label at its whole width.
        #expect(fresh.item > fresh.label)
        let changed = try #require(try await Self.widths(when))
        #expect(changed == fresh, "changed \(when): \(changed); from the start: \(fresh)")
    }
}
