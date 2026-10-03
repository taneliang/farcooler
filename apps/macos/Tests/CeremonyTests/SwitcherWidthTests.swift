import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The title bar's switcher reads its whole label, "Main · overnight", in
/// the window as the app wires its toolbar (ov-177: the owner saw
/// "Main · ov…"). Drawn in a real titled window, offscreen, with the
/// toolbar's other items beside it, and the label's width read from the
/// view the switcher's menu drops from, which is exactly the label.
///
/// A guard, not a reproduction: offscreen, neither the label without
/// `fixedSize` nor ov-105's grouped layout (a sidebar button and the
/// switcher in one glass group) truncated it, at 1000, 700 or 420 pt, nor
/// when the repository's name arrived after the first layout. The owner's
/// "Main · ov…" is confirmed in the live app.
@MainActor
@Suite(.serialized)
struct SwitcherWidthTests {
    /// What the window names: at first nothing read yet, then the
    /// workspace, as a window opening does.
    @MainActor
    final class Place: ObservableObject {
        @Published var title = "Main"
        @Published var repository = ""
    }

    struct Toolbar: View {
        @ObservedObject var place: Place
        var body: some View {
            NavigationSplitView(columnVisibility: .constant(.all)) {
                List { Text("Needs You") }
            } detail: {
                Color.clear
                    .toolbar {
                        TrailingToolbar(
                            troubles: [], stale: [], updates: [], needsYou: 3, needsYouSelected: false,
                            onNeedsYou: {}, perform: { _ in })
                    }
                    .toolbar {
                        ToolbarItem { Button {} label: { Label("Changes", systemImage: "plusminus") } }
                        ToolbarItem { Button {} label: { Label("Open in Editor", systemImage: "arrow.up.forward.app") } }
                    }
                    .toolbar(removing: .title)
                    .toolbar {
                        LeadingToolbar(
                            switcher: WorkspaceSwitcherButton(
                                title: place.title, repository: place.repository, entries: [], openRequest: 0,
                                perform: { _ in }))
                    }
            }
        }
    }

    static let switcher = WorkspaceSwitcherButton(
        title: "Main", repository: "overnight", entries: [], openRequest: 0, perform: { _ in })

    /// The menu anchors under `view`, by their widths.
    static func anchors(in view: NSView) -> [CGFloat] {
        let mine = String(describing: type(of: view)) == "Anchor" ? [view.frame.width] : []
        return mine + view.subviews.flatMap(anchors)
    }

    /// The label's width with all the room it wants.
    static func idealWidth() async -> CGFloat {
        let host = NSHostingView(rootView: switcher.fixedSize())
        let window = NSWindow(
            contentRect: NSRect(x: -6000, y: -6000, width: 600, height: 60), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        defer { window.close() }
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return anchors(in: host).first ?? 0
    }

    @Test("The switcher shows its whole label in the title bar", arguments: [1000, 700, 480] as [CGFloat])
    func theWholeLabel(width: CGFloat) async throws {
        let ideal = await Self.idealWidth()
        #expect(ideal > 80, "the label measured \(ideal) pt with all the room it wants")
        let window = NSWindow(
            contentRect: NSRect(x: -6000, y: -6000, width: width, height: 300),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.toolbarStyle = .unified
        let place = Place()
        window.contentViewController = NSHostingController(rootView: Toolbar(place: place))
        window.setContentSize(NSSize(width: width, height: 300))
        window.orderFrontRegardless()
        defer { window.close() }
        for _ in 0..<15 {
            window.contentView?.superview?.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(40))
        }
        // The repository's name arrives once its runner has listed it.
        place.repository = "overnight"
        for _ in 0..<25 {
            window.contentView?.superview?.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(40))
        }
        let drawn = Self.anchors(in: window.contentView!.superview!)
        #expect(drawn.count == 1, "found \(drawn.count) switchers in the title bar")
        for found in drawn {
            #expect(found >= ideal - 0.5, "the label got \(found) pt of the \(ideal) it needs at \(width) pt")
        }
    }
}
