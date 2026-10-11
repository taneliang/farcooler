import AppKit
import QuietTestProcess

/// The one way a test puts a window on screen (ov-473).
///
/// Tests used to call `makeKeyAndOrderFront` and `orderFrontRegardless`,
/// which put real windows over whatever the person was doing and could take
/// the frontmost spot. A window shown here is
///
/// - behind every app's windows, one level below normal ones, so it never
///   covers the person's work;
/// - deaf to the mouse, so a click meant for what is under it lands there;
/// - in a process that can never be activated (`QuietTestProcess`), so the
///   person's app stays frontmost and keeps the keyboard.
///
/// It stays on a real screen and opaque, because tests rely on both. On a
/// 120 Hz display a window ordered behind everything gets 120 to 123
/// `NSView.displayLink` callbacks a second, even under the front app's
/// window, where one moved off every screen gets none; the animated scrolls
/// of ov-429 need those ticks. And the tests that ask the window server for
/// a window's pixels (`screencapture -l`) get nothing from a transparent
/// one. A window a test placed off every screen stays there.
///
/// The window is made the one its app would make key; it is never actually
/// key, because a window can only be key in an active app, and the one kind
/// that can be key without activating (a nonactivating panel) takes the
/// keyboard from the person's app. Tests that send keys send them to the
/// window, which needs no key status.
public enum TestWindow {
    /// Below every normal window of every app.
    public static let level = NSWindow.Level(rawValue: NSWindow.Level.normal.rawValue - 1)

    /// Put `window` on screen behind everything, without activating anything.
    @MainActor public static func show(_ window: NSWindow) {
        farcooler_quiet_test_process()
        window.ignoresMouseEvents = true
        window.animationBehavior = .none
        window.level = level
        window.orderBack(nil)
        window.makeKey()
    }
}
