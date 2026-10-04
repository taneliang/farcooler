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

    nonisolated static let cases: [(CGFloat, TitleStatus.Form, Bool)] = [
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
        if let control { try Self.expectAfterTheSwitcher(control, status: status, in: window, at: width) }
    }

    /// Back and Forward in the leading group: just after the switcher, and
    /// before the status area, never at the trailing end (integ-8: a
    /// ControlGroup inserted on screen landed past the center item). Frames,
    /// with a gap allowance far smaller than the error it catches (the
    /// trailing end is ~1,000 pt away), not exact text widths.
    static func expectAfterTheSwitcher(
        _ control: CGRect, status: TitleBarHarness.Status, in window: NSWindow, at width: CGFloat
    ) throws {
        let switcher = try #require(Harness.switcher(in: window))
        #expect(control.minX >= switcher.maxX, "at \(width): before the switcher")
        #expect(control.minX - switcher.maxX < 48, "at \(width): \(control.minX - switcher.maxX) pt past the switcher")
        #expect(control.maxX <= status.frame.minX, "at \(width): past the status area")
        #expect(control.maxX < width / 2, "at \(width): in the trailing half")
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
            // Inserted once the window is on screen: the case integ-8 caught.
            if let control = Harness.backForward(in: window) {
                try Self.expectAfterTheSwitcher(control, status: status, in: window, at: width)
            }
        }
    }

    @Test("Their tooltips name the keys that do the same")
    func tooltips() {
        #expect(BackForwardControl.backHelp == "Go back (⌃⌘←)")
        #expect(BackForwardControl.forwardHelp == "Go forward (⌃⌘→)")
    }
}

/// The mouse's side buttons and the swipe between pages go back and forward
/// (ov-214), and nothing else does.
struct BackForwardGestureTests {
    @Test("Button 4 goes back, button 5 forward; a swipe goes the way it's swiped; nothing else navigates")
    func directions() {
        #expect(BackForwardGesture.direction(type: .otherMouseDown, button: 3) == .back)
        #expect(BackForwardGesture.direction(type: .otherMouseDown, button: 4) == .forward)
        #expect(BackForwardGesture.direction(type: .otherMouseDown, button: 2) == nil, "the middle button")
        #expect(BackForwardGesture.direction(type: .swipe, deltaX: 1) == .back)
        #expect(BackForwardGesture.direction(type: .swipe, deltaX: -1) == .forward)
        #expect(BackForwardGesture.direction(type: .swipe, deltaX: 0) == nil, "a vertical swipe")
        #expect(BackForwardGesture.direction(type: .leftMouseDown, button: 3) == nil)
        #expect(BackForwardGesture.direction(type: .scrollWheel, deltaX: 5) == nil)
    }
}
