import SwiftUI

/// What the main window says about itself to the menu bar, while it is the key
/// window.
///
/// A menu item's key equivalent reaches whatever window is key. Close Terminal
/// posts a notification the main window's `ContentView` acts on, so with the
/// Settings window key, ⌘W stopped and removed the selected terminal behind it
/// and left Settings open. The menu bar cannot tell which window is key on its
/// own; this is how it learns. `ContentView` publishes it as a focused SCENE
/// value, so it is nil whenever any other window (Settings, About) is key, and
/// the items that act on the main window are disabled then. A disabled item's
/// chord goes on to the next match, which for ⌘W is File ▸ Close.
struct MainWindowFocus: Equatable {
    /// The ⌘N task panel or the ⌘P palette is open over the window.
    var overlayOpen: Bool

    /// Close Terminal (⌘W) acts only when the main window is key.
    static func closesTerminal(_ focus: MainWindowFocus?) -> Bool {
        focus != nil
    }

    /// Zoom Pane (⇧⌘↩) acts only when the main window is key and nothing is
    /// open over it. The ⌘N panel and the ⌘P palette each have their own
    /// meaning for ⇧⌘↩ (a newline, and submit), and an enabled menu item would
    /// take the chord before either field saw it, zooming a pane nobody can
    /// see behind the overlay.
    static func zoomsPane(_ focus: MainWindowFocus?) -> Bool {
        guard let focus else { return false }
        return !focus.overlayOpen
    }
}

extension FocusedValues {
    @Entry var mainWindow: MainWindowFocus?
}
