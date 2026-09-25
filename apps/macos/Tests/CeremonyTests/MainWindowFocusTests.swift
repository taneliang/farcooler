import Testing

@testable import Far_Cooler

/// Which menu items act, depending on which window is key.
///
/// ⌘W in the Settings window stopped and removed the selected terminal behind
/// it, because Close Terminal was enabled whatever window was key. The menu bar
/// now learns from `MainWindowFocus`, which only the main window publishes.
struct MainWindowFocusTests {
    @Test("Close Terminal acts only while the main window is key")
    func closeTerminalActsOnlyInTheMainWindow() {
        #expect(MainWindowFocus.closesTerminal(MainWindowFocus(overlayOpen: false)))
        // Settings, About, any other window: nothing published.
        #expect(!MainWindowFocus.closesTerminal(nil))
    }

    @Test("Zoom Pane acts only in the main window with nothing open over it")
    func zoomPaneActsOnlyWithNothingOverTheWindow() {
        #expect(MainWindowFocus.zoomsPane(MainWindowFocus(overlayOpen: false)))
        // The ⌘N panel or the ⌘P palette: ⇧⌘↩ is theirs.
        #expect(!MainWindowFocus.zoomsPane(MainWindowFocus(overlayOpen: true)))
        #expect(!MainWindowFocus.zoomsPane(nil))
    }
}
