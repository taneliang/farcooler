import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The title bar's status area is in the toolbar at every window width,
/// whole, in the form that width affords, and never pushes the tray into the
/// overflow menu; and it's measured again whenever what it draws changes
/// size (ov-214).
///
/// The toolbar sizes an item once, when the window's content is installed,
/// and doesn't flex a center item (ov-177; the design's probe). The area
/// holds one fixed width per form and is a new view when the form changes,
/// so these are the ways it could still go wrong: a form too wide for the
/// room (the trailing items overflow), a form chosen before the window's
/// width is known and never replaced (the item stays at that width), and
/// text that grows inside a form (clipped, or the item resized under it).
@MainActor
@Suite(.serialized)
struct TitleStatusWidthTests {
    typealias Harness = TitleBarHarness

    private static func root(_ words: Harness.Words) -> Harness.Root<Color> {
        Harness.Root(words: words, content: Color.clear)
    }

    /// The window widths the owner uses and the window's minimum, with the
    /// form each affords beside "Main · overnight", Open in Editor, Changes
    /// and a tray reading 11.
    nonisolated static let widths: [(CGFloat, TitleStatus.Form)] = [
        (1790, .wide), (1200, .wide), (900, .medium), (700, .short), (600, .ring),
    ]

    @Test("The status area is whole, in its form, and leaves every other item in place", arguments: widths)
    func fitsAtEveryWidth(width: CGFloat, form: TitleStatus.Form) async throws {
        // Everything at the widest, for the count of items a window shows
        // when nothing has gone to the overflow menu.
        let wide = try await Harness.window(Self.root(Harness.Words()), width: 1790)
        let everything = Harness.itemsShown(in: wide)
        wide.close()

        let window = try await Harness.window(Self.root(Harness.Words()), width: width)
        defer { window.close() }
        let status = try #require(Harness.status(in: window), "no status area in the toolbar at \(width)")
        #expect(status.width == form.width, "at \(width): \(status.width) wide, not \(form)'s \(form.width); window \(window.frame), content \(window.contentLayoutRect), screens \(NSScreen.screens.map(\.frame))")
        #expect(status.item >= status.width, "at \(width): the item is \(status.item), its content \(status.width)")
        #expect(Harness.itemsShown(in: window) == everything, "at \(width) an item went to the overflow menu")
        let switcher = try #require(Harness.switcher(in: window))
        #expect(status.frame.minX >= switcher.maxX, "at \(width) the status area overlaps the switcher")
        #expect(status.frame.maxX <= width, "at \(width) the status area runs off the window")
        #expect(Harness.band(window) == 40, "the toolbar is \(Harness.band(window)) pt, not the compact 40")
    }

    /// When the text grows: before the window shows (after the toolbar
    /// measured the item) or once it's on screen.
    enum When: CustomStringConvertible {
        case beforeShowing, onScreen
        var description: String { self == .beforeShowing ? "before showing" : "on screen" }
    }

    @Test("A longer line and a bigger count don't change the item's width", arguments: [When.beforeShowing, .onScreen])
    func textGrowing(when: When) async throws {
        let fresh = Harness.Words()
        let before = try await Harness.window(Self.root(fresh), width: 900)
        let measured = try #require(Harness.status(in: before))
        before.close()

        let words = Harness.Words()
        func grow() {
            words.nowDoing = "Reading ov-192’s jump bar diff, then the integration report for integ-4 and its captures"
            words.needYou = 142
        }
        let window = try await Harness.window(Self.root(words), width: 900, beforeShowing: when == .beforeShowing ? grow : {})
        defer { window.close() }
        if when == .onScreen {
            grow()
            try await Harness.settle(window)
        }
        let grown = try #require(Harness.status(in: window))
        #expect(grown.width == measured.width && grown.item == measured.item, "grown \(when): \(grown); fresh: \(measured)")
        #expect(grown.item >= grown.width)
    }

    /// The form follows the window: made wide, then narrowed on screen, the
    /// item is measured again at the new form's width, and the tray stays.
    @Test("Narrowing the window on screen measures the status area again")
    func narrowingOnScreen() async throws {
        let window = try await Harness.window(Self.root(Harness.Words()), width: 1790)
        defer { window.close() }
        let everything = Harness.itemsShown(in: window)
        #expect(Harness.status(in: window)?.width == TitleStatus.Form.wide.width)
        for (width, form) in [(900, TitleStatus.Form.medium), (600, .ring), (1200, .wide)] as [(CGFloat, TitleStatus.Form)] {
            window.setContentSize(NSSize(width: width, height: 400))
            try await Harness.settle(window)
            let status = try #require(Harness.status(in: window))
            #expect(status.width == form.width && status.item >= status.width, "at \(width): \(status)")
            #expect(Harness.itemsShown(in: window) == everything, "at \(width) an item went to the overflow menu")
        }
    }

    /// A long workspace name takes room from the area, not the tray's.
    @Test("A long switcher label takes a narrower form, not the tray")
    func longSwitcherLabel() async throws {
        let words = Harness.Words()
        words.title = "Billing reconciliation rewrite"
        words.repository = "shop-frontend-monorepo"
        let window = try await Harness.window(Self.root(words), width: 900)
        defer { window.close() }
        let status = try #require(Harness.status(in: window))
        let expected = TitleStatusRoom(
            switcherTitle: words.title, switcherRepository: words.repository, editor: true, changes: true,
            trouble: nil, needsYou: 11
        ).form(window: 900)
        #expect(expected < .medium, "the long label left room for \(expected)")
        #expect(status.width == expected.width)
        let switcher = try #require(Harness.switcher(in: window))
        #expect(status.frame.minX >= switcher.maxX, "the status area overlaps the switcher")
        // The tray, the editor and Changes all still shown.
        let wide = try await Harness.window(Self.root(words), width: 1790)
        let everything = Harness.itemsShown(in: wide)
        wide.close()
        #expect(Harness.itemsShown(in: window) == everything)
    }
}

/// The field opens in the status area's own width (slice 4), so the toolbar
/// never has to measure it again; too narrow for a field, it opens in the
/// panel under the bar instead, and the item stays as it was.
@MainActor
@Suite(.serialized)
struct TitleConsoleWidthTests {
    typealias Harness = TitleBarHarness

    @Test(
        "Opening and closing the field leaves the status item's width alone",
        arguments: [(CGFloat(1790), TitleStatus.Form.wide), (900, .medium), (600, .ring)])
    func openingKeepsTheWidth(width: CGFloat, form: TitleStatus.Form) async throws {
        let console = TitleConsoleModel()
        let window = try await Harness.window(
            Harness.Root(words: Harness.Words(), console: console, content: Color.clear), width: width)
        defer { window.close() }
        let closed = try #require(Harness.status(in: window))
        #expect(closed.width == form.width)
        for finding in [false, true] {
            console.console.open(finding: finding)
            try await Harness.settle(window)
            let open = try #require(Harness.status(in: window), "the status item left the toolbar")
            #expect(open.width == closed.width && open.item == closed.item, "open (finding \(finding)): \(open)")
            console.console.close()
            try await Harness.settle(window)
        }
        #expect(Harness.status(in: window) == closed)
    }
}
