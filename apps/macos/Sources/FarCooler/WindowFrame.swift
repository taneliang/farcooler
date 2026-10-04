import AppKit
import Foundation

/// Where a window sits on screen, kept in its record and put back on a
/// relaunch (ov-233). AppKit's own `setFrame(from:)` would do the arithmetic
/// for a descriptor, but it keeps a window on a screen only loosely, and a
/// display that's gone since can strand one off every screen; so the rule is
/// written here, over a list of screens a test can give it.
enum WindowFrame {
    /// The frame a descriptor holds: `NSWindow.frameDescriptor` starts with
    /// the frame, `x y width height`, and goes on with the screen it was on.
    static func rect(from descriptor: String) -> CGRect? {
        let numbers = descriptor.split(separator: " ").prefix(4).compactMap { Double($0) }
        guard numbers.count == 4, numbers[2] > 0, numbers[3] > 0 else { return nil }
        return CGRect(x: numbers[0], y: numbers[1], width: numbers[2], height: numbers[3])
    }

    /// How much of its title bar must be on a screen for a window to be
    /// reachable: enough to grab.
    static let reachable = CGSize(width: 100, height: 20)
    private static let titleBar: CGFloat = 40

    /// `frame` where a window can be put: itself while its title bar is on one
    /// of `screens` (their visible frames, the one with the menu bar first),
    /// else the same size, cut to fit, in the middle of the first.
    static func placed(_ frame: CGRect, on screens: [CGRect]) -> CGRect {
        guard let main = screens.first else { return frame }
        let strip = CGRect(x: frame.minX, y: frame.maxY - titleBar, width: frame.width, height: titleBar)
        let grabbable = screens.contains {
            let shared = $0.intersection(strip)
            return !shared.isNull && shared.width >= reachable.width && shared.height >= reachable.height
        }
        if grabbable { return frame }
        let size = CGSize(width: min(frame.width, main.width), height: min(frame.height, main.height))
        return CGRect(
            x: main.midX - size.width / 2, y: main.midY - size.height / 2, width: size.width, height: size.height)
    }

    /// Put `window` where its record says, and into full screen if it was.
    @MainActor
    static func apply(_ session: WindowSession, to window: NSWindow) {
        if let frame = session.frame.flatMap(rect(from:)) {
            window.setFrame(placed(frame, on: NSScreen.screens.map(\.visibleFrame)), display: true)
        }
        if session.fullScreen, !window.styleMask.contains(.fullScreen) {
            DispatchQueue.main.async { window.toggleFullScreen(nil) }
        }
    }

    /// The notifications that say a window moved, was resized or changed
    /// screen or full-screen state.
    static let changes: [Notification.Name] = [
        NSWindow.didEndLiveResizeNotification, NSWindow.didMoveNotification, NSWindow.didChangeScreenNotification,
        NSWindow.didEnterFullScreenNotification, NSWindow.didExitFullScreenNotification,
    ]

    // MARK: - What SwiftUI kept

    /// The frames SwiftUI saved under a key made from the view's generated
    /// type name, which changes with every modifier added to the window's
    /// root: one stale key for each, the first window's frame lost to each.
    static func staleKeys(in keys: [String]) -> [String] {
        keys.filter { $0.hasPrefix("NSWindow Frame SwiftUI.")
                && ($0.contains("Far_Cooler.ContentView") || $0.contains("FarCooler.ContentView")) }
    }

    /// Remove them, whenever there are any: a record keeps each window's frame
    /// now, and the next change to the root's modifiers makes another.
    static func removeStale(from defaults: UserDefaults = .standard) {
        for key in staleKeys(in: Array(defaults.dictionaryRepresentation().keys)) { defaults.removeObject(forKey: key) }
    }
}
