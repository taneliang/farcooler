import AppKit

/// What ⌘W closes (ov-265): the selected terminal while the main window is
/// key, else the window that is, Settings or About.
///
/// Close Terminal holds ⌘W, and SwiftUI takes the chord off File ▸ Close
/// for it. Disabled with Settings key, as it was, the item still answered
/// ⌘W, closing nothing: a disabled menu item with a key equivalent swallows
/// the chord, and Close had none to fall through to. So the one ⌘W item
/// acts in every window, as a closer of whichever is key.
enum CloseCommand: Equatable {
    case terminal
    case window

    /// ⌘W with `focus` published: the main window's, or nil when another
    /// window is key.
    static func at(_ focus: MainWindowFocus?) -> CloseCommand {
        MainWindowFocus.closesTerminal(focus) ? .terminal : .window
    }

    var title: String {
        switch self {
        case .terminal: "Close Terminal"
        case .window: "Close Window"
        }
    }

    /// Do it: `keyWindow` is the window that's key, `NSApp.keyWindow`.
    @MainActor
    func perform(keyWindow: NSWindow?) {
        switch self {
        case .terminal: AppCommand.closeTerminal.post()
        case .window: keyWindow?.performClose(nil)
        }
    }

    /// File ▸ Close All (⌥⌘W), which replacing File's close items takes
    /// away with Close: every window that can be closed.
    @MainActor
    static func closeAll(_ windows: [NSWindow]) {
        for window in windows where window.isVisible && window.styleMask.contains(.closable) {
            window.performClose(nil)
        }
    }
}
