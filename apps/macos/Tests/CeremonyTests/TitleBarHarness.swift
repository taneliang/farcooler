import AgentKit
import AppKit
import SwiftUI

@testable import Far_Cooler

/// A real titled window with the main window's title bar, as `ContentView`
/// wires it (ov-214): the leading group, the status area, Open in Editor and
/// Changes, the tray, and the window's chrome. Off every screen, so a run
/// draws nothing on the owner's, and driven only through its model: nothing
/// here sends input.
///
/// A real window, because only a window's toolbar measures items and lays
/// them out, and only a titled window has a toolbar to measure.
@MainActor
enum TitleBarHarness {
    /// What the title bar says, changed from outside as the window's store
    /// would change it.
    @MainActor @Observable final class Words {
        var nowDoing: String? = "Reading the board"
        var needYou = 3
        var title = "Main"
        var repository = "overnight"
    }

    static let worktree = Worktree(
        id: "co", short: "co", task: "overnight", branch: "main", repository: "overnight", host: "",
        path: "/tmp/overnight", state: "active", terminals: [])

    /// The window's title bar over `content`.
    struct Root<Content: View>: View {
        let words: Words
        /// The tray's count, every workspace's.
        var trayCount = 11
        let content: Content

        var body: some View {
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .background(WorkspaceStyle.canvas)
                .toolbar {
                    TrailingToolbar(
                        troubles: [], stale: [], updates: [], needsYou: trayCount, needsYouSelected: false,
                        onNeedsYou: {}, perform: { _ in })
                }
                .toolbar {
                    WorktreeToolbar(
                        editor: TitleBarHarness.worktree, onEditorError: { _ in },
                        changes: .init(worktree: TitleBarHarness.worktree, open: false), onChanges: { _ in })
                }
                .titleBarStatus(
                    TitleStatusSource(
                        orchestrator: .working, status: .working, nowDoing: words.nowDoing,
                        waiting: { [needYou = words.needYou] _ in needYou }),
                    room: TitleStatusRoom(
                        switcherTitle: words.title, switcherRepository: words.repository, editor: true,
                        changes: true, trouble: nil, needsYou: trayCount),
                    actions: TitleStatusActions())
                .toolbar(removing: .title)
                .toolbar {
                    LeadingToolbar(
                        switcher: WorkspaceSwitcherButton(
                            title: words.title, repository: words.repository, entries: [], openRequest: 0,
                            perform: { _ in }),
                        navigator: NavigatorToggle(hidden: false, available: true, toggle: {}))
                }
                .mainWindowChrome()
        }
    }

    /// A window `width` wide around `root`, made the way SwitcherWidthTests
    /// makes one: titled, full-size content, starting in the unified style,
    /// so the chrome the root asks for is what changes it. `beforeShowing`
    /// runs after the content is installed and before the window is on
    /// screen: the launch race (ov-177).
    static func window<V: View>(
        _ root: V, width: CGFloat, height: CGFloat = 400, beforeShowing: () -> Void = {}
    ) async throws -> NSWindow {
        let window = NSWindow(
            contentRect: NSRect(x: -6000, y: -6000, width: width, height: height),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.toolbarStyle = .unified
        window.contentViewController = NSHostingController(rootView: root)
        window.setContentSize(NSSize(width: width, height: height))
        beforeShowing()
        window.orderFrontRegardless()
        try await settle(window)
        return window
    }

    static func settle(_ window: NSWindow) async throws {
        for _ in 0..<15 {
            window.contentView?.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(40))
        }
    }

    /// The window's frame from its top to its content's: the toolbar's band.
    static func band(_ window: NSWindow) -> CGFloat {
        window.frame.height - window.contentLayoutRect.height
    }

    /// The status area as laid out: its own width, the width of the toolbar
    /// item holding it, and where it is in the window.
    struct Status: Equatable {
        let width: CGFloat
        let item: CGFloat
        let frame: CGRect
    }

    static func status(in window: NSWindow) -> Status? {
        guard let anchor = first(TitleStatusAnchor.View.self, in: window.contentView?.superview) else { return nil }
        guard let item = hostingItem(of: anchor) else { return nil }
        return Status(width: anchor.frame.width, item: item.frame.width, frame: anchor.convert(anchor.bounds, to: nil))
    }

    /// The switcher's frame in the window: its menu anchor's.
    static func switcher(in window: NSWindow) -> CGRect? {
        guard let anchor = first(named: "Anchor", in: window.contentView?.superview) else { return nil }
        return anchor.convert(anchor.bounds, to: nil)
    }

    /// How many toolbar items are laid out in the window, the overflow
    /// menu's not counted: one moved into the overflow leaves the window.
    static func itemsShown(in window: NSWindow) -> Int {
        var count = 0
        func walk(_ view: NSView) {
            if String(describing: type(of: view)).hasPrefix("ToolbarItemHostingView"), !view.isHidden,
                view.frame.width > 0
            {
                count += 1
            }
            for sub in view.subviews { walk(sub) }
        }
        if let root = window.contentView?.superview { walk(root) }
        return count
    }

    private static func hostingItem(of view: NSView) -> NSView? {
        var item = view.superview
        while let current = item, !String(describing: type(of: current)).hasPrefix("ToolbarItemHostingView") {
            item = current.superview
        }
        return item
    }

    private static func first<T: NSView>(_ type: T.Type, in view: NSView?) -> T? {
        guard let view else { return nil }
        if let found = view as? T { return found }
        for sub in view.subviews { if let found = first(type, in: sub) { return found } }
        return nil
    }

    private static func first(named name: String, in view: NSView?) -> NSView? {
        guard let view else { return nil }
        if String(describing: Swift.type(of: view)) == name { return view }
        for sub in view.subviews { if let found = first(named: name, in: sub) { return found } }
        return nil
    }
}
