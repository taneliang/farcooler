import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Back and Forward in the title bar (ov-214, slice 3): drawn where the
/// status area keeps its medium form beside them, and given up before the
/// status area is squeezed; never pushing an item into the overflow menu.
@MainActor
@Suite(.serialized)
struct BackForwardToolbarTests {
    typealias Harness = TitleBarHarness

    private static let room = TitleStatusRoom(
        switcherTitle: "Main", switcherRepository: "overnight", editor: true, changes: true, trouble: nil,
        needsYou: 11, backForward: true)

    static let cases: [(CGFloat, TitleStatus.Form, Bool)] = [
        (1790, .wide, true), (1200, .medium, true), (900, .medium, false), (600, .ring, false),
    ]

    @Test("Back and Forward are drawn only beside a medium or wider status area", arguments: cases)
    func layout(width: CGFloat, form: TitleStatus.Form, shown: Bool) {
        let layout = Self.room.layout(window: width)
        #expect(layout.form == form, "at \(width)")
        #expect(layout.backForward == shown, "at \(width)")
        var without = Self.room
        without.backForward = false
        #expect(!without.layout(window: width).backForward, "a window without them never draws them")
    }

    private static func root(backForward: Bool) -> Harness.Root<Color> {
        Harness.Root(words: Harness.Words(), backForward: backForward, content: Color.clear)
    }

    @Test("In a real window, Back and Forward sit after the switcher and push nothing into the overflow menu", arguments: cases)
    func inTheWindow(width: CGFloat, form: TitleStatus.Form, shown: Bool) async throws {
        let all = try await Harness.window(Self.root(backForward: true), width: 1790)
        let withThem = Harness.itemsShown(in: all)
        all.close()
        let bare = try await Harness.window(Self.root(backForward: false), width: 1790)
        let withoutThem = Harness.itemsShown(in: bare)
        bare.close()
        #expect(withThem > withoutThem)

        let window = try await Harness.window(Self.root(backForward: true), width: width)
        defer { window.close() }
        let control = Harness.backForward(in: window)
        #expect((control != nil) == shown, "at \(width)")
        #expect(Harness.itemsShown(in: window) == (shown ? withThem : withoutThem), "at \(width) an item overflowed")
        let status = try #require(Harness.status(in: window))
        #expect(status.width == form.width, "at \(width)")
        if let control, let switcher = Harness.switcher(in: window) {
            #expect(control.minX >= switcher.maxX)
            #expect(status.frame.minX >= control.maxX, "the status area overlaps Back and Forward")
        }
    }

    @Test("Widening and narrowing the window on screen brings Back and Forward in and takes them out")
    func resizingOnScreen() async throws {
        let window = try await Harness.window(Self.root(backForward: true), width: 900)
        defer { window.close() }
        #expect(Harness.backForward(in: window) == nil)
        for (width, shown, form) in [(1790, true, TitleStatus.Form.wide), (900, false, .medium), (1200, true, .medium)]
            as [(CGFloat, Bool, TitleStatus.Form)]
        {
            window.setContentSize(NSSize(width: width, height: 400))
            try await Harness.settle(window)
            #expect((Harness.backForward(in: window) != nil) == shown, "at \(width)")
            let status = try #require(Harness.status(in: window))
            #expect(status.width == form.width && status.item >= status.width, "at \(width): \(status)")
        }
    }

    @Test("Their tooltips name the keys that do the same")
    func tooltips() {
        #expect(BackForwardControl.backHelp == "Go back (⌃⌘←)")
        #expect(BackForwardControl.forwardHelp == "Go forward (⌃⌘→)")
    }
}
