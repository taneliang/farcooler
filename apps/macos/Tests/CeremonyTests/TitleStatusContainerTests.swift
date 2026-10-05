import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The title bar's glass follows the HIG (ov-291): the status area sits on no
/// glass (ov-263 reversed), the switcher is the title as text, and the glass
/// that remains is three groups.
@MainActor
@Suite(.serialized)
struct TitleStatusContainerTests {
    typealias Harness = TitleBarHarness

    /// The class names from `view` up to the window's frame.
    private static func ancestry(_ view: NSView?) -> [String] {
        var names: [String] = []
        var current = view
        while let next = current {
            names.append(String(describing: type(of: next)))
            current = next.superview
        }
        return names
    }

    private static func statusItem(in window: NSWindow) -> NSView? {
        func find(_ view: NSView) -> NSView? {
            if view is TitleStatusAnchor.View { return view }
            for sub in view.subviews { if let found = find(sub) { return found } }
            return nil
        }
        return window.contentView?.superview.flatMap(find)
    }

    /// Every `NSToolbarPlatterView` under `view`: one per glass group.
    private static func platters(in view: NSView?) -> [NSView] {
        guard let view else { return [] }
        let own = String(describing: type(of: view)) == "NSToolbarPlatterView" ? [view] : []
        return own + view.subviews.flatMap { platters(in: $0) }
    }

    private static func platter(of view: NSView?) -> NSView? {
        ancestors(view).first { String(describing: type(of: $0)) == "NSToolbarPlatterView" }
    }

    private static func ancestors(_ view: NSView?) -> [NSView] {
        var out: [NSView] = []
        var current = view
        while let next = current { out.append(next); current = next.superview }
        return out
    }

    /// The toolbar items drawn under `view`.
    private static func itemViewers(in view: NSView) -> Int {
        (String(describing: type(of: view)) == "NSToolbarItemViewer" ? 1 : 0)
            + view.subviews.reduce(0) { $0 + itemViewers(in: $1) }
    }

    /// The center status sits on no glass (ov-291, reversing ov-263): Apple's
    /// example of a principal status item hides the shared background.
    @Test("The status area is on the bar, under no glass", arguments: [1400, 900, 700])
    func onNoGlass(width: CGFloat) async throws {
        let root = Harness.Root(words: Harness.Words(), backForward: true, content: Color.clear)
        let window = try await Harness.window(root, width: width)
        defer { window.close() }
        let status = Self.ancestry(try #require(Self.statusItem(in: window), "no status area at \(width)"))
        #expect(!status.contains("NSGlassEffectView"), "at \(width), glass under it: \(status)")
        #expect(!status.contains("NSToolbarPlatterView"), "at \(width), on a platter: \(status)")
    }

    /// Four platters: the sidebar button with Back and Forward, Open in
    /// Editor (a split button, which AppKit always sets apart), Changes, and the
    /// tray: HIG's three groups, and the system's own split of the editor's menu
    /// (research, ov-291). The switcher, a text button, and the status are on
    /// none. Before ov-291 there were six, with the switcher and the status
    /// among them.
    @Test("The toolbar has four glass groups; the sidebar button shares one with Back and Forward")
    func glassGroups() async throws {
        let root = Harness.Root(words: Harness.Words(), backForward: true, content: Color.clear)
        let window = try await Harness.window(root, width: 1400)
        defer { window.close() }
        let platters = Self.platters(in: window.contentView?.superview)
        #expect(platters.count == 4, "\(platters.count) glass groups: \(Harness.outline(of: window.contentView?.superview))")
        let back = try #require(Harness.backForwardItem(in: window), "no Back and Forward")
        let leading = try #require(Self.platter(of: back), "Back and Forward are on no platter")
        #expect(Self.itemViewers(in: leading) == 2, "the sidebar button and Back and Forward share one platter")
        let switcher = try #require(Harness.switcherItem(in: window), "no switcher")
        #expect(Self.platter(of: switcher) == nil, "the switcher, text, is on a platter")
        // The title follows the leading controls.
        let toggleEnd = leading.convert(leading.bounds, to: nil).maxX
        #expect(switcher.convert(switcher.bounds, to: nil).minX >= toggleEnd - 0.5, "the switcher is before Back and Forward")
    }

    @Test("Nothing is drawn in the capsule's ends now")
    func insets() {
        #expect(TitleStatus.capsuleInset(.wide) == 0 && TitleStatus.capsuleInset(.ring) == 0)
    }
}
