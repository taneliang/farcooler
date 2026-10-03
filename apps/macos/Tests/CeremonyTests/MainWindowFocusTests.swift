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

    @Test("Next Needing Attention acts only while the main window is key, with something waiting")
    func nextAttentionActsOnlyInTheMainWindow() {
        var waiting = MainWindowFocus(overlayOpen: false)
        waiting.hasAttention = true
        #expect(MainWindowFocus.stepsToAttention(waiting))
        // Nothing waiting: ⌃⌘N did nothing, and is dimmed (ov-211).
        #expect(!MainWindowFocus.stepsToAttention(MainWindowFocus(overlayOpen: false)))
        #expect(!MainWindowFocus.stepsToAttention(nil))
    }

    @Test("Zoom Pane acts only in the main window with nothing open over it")
    func zoomPaneActsOnlyWithNothingOverTheWindow() {
        func focus(overlay: Bool) -> MainWindowFocus {
            var focus = MainWindowFocus(overlayOpen: overlay)
            focus.layout = LayoutMenuFocus(panes: 2)
            return focus
        }
        #expect(MainWindowFocus.zoomsPane(focus(overlay: false)))
        // The ⌘N panel or the ⌘P palette: ⇧⌘↩ is theirs.
        #expect(!MainWindowFocus.zoomsPane(focus(overlay: true)))
        #expect(!MainWindowFocus.zoomsPane(nil))
        // One pane has nothing to zoom over, and no layout nothing to zoom.
        var one = focus(overlay: false)
        one.layout = LayoutMenuFocus(panes: 1)
        #expect(!MainWindowFocus.zoomsPane(one))
        #expect(!MainWindowFocus.zoomsPane(MainWindowFocus(overlayOpen: false)))
    }
}
