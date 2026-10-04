import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The title bar's status area sits in a container of its own (ov-263): the
/// toolbar's glass capsule, as Xcode's activity area and Safari's address
/// field do, rather than bare text between the leading and trailing capsules.
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

    @Test("The status area is in the toolbar's glass capsule, as Back and Forward are", arguments: [1400, 900, 700])
    func inACapsule(width: CGFloat) async throws {
        let root = Harness.Root(words: Harness.Words(), backForward: true, content: Color.clear)
        let window = try await Harness.window(root, width: width)
        defer { window.close() }
        let status = Self.ancestry(try #require(Self.statusItem(in: window), "no status area at \(width)"))
        #expect(status.contains("NSToolbarPlatterView"), "at \(width), bare on the bar: \(status)")
        #expect(status.contains("NSGlassEffectView"), "at \(width), no glass under it: \(status)")
    }

    @Test("Its parts sit clear of the capsule's ends, the ring's by less")
    func insets() {
        #expect(TitleStatus.capsuleInset(.wide) == 12 && TitleStatus.capsuleInset(.medium) == 12)
        #expect(TitleStatus.capsuleInset(.ring) == 8)
    }
}
