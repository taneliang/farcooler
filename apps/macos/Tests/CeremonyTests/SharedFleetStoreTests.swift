import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// One `FleetStore` for every window (ov-133): the same runners, the same
/// streams, one wake and retry hook, and each window's commands its own.
@MainActor
struct SharedFleetStoreTests {
    /// Two windows' views hold the app's one store, so `Reachability`'s one
    /// hook, set by that store, reaches both. Each window built its own, and
    /// the newest took the hook from the rest.
    @Test func everyWindowHoldsTheAppsOneStore() {
        let first = ContentView(), second = ContentView()
        #expect(first.store === second.store)
        #expect(first.store === FleetStore.shared)
    }

    /// Closing one of two windows leaves the streams running for the other;
    /// closing the last stops them.
    @Test func onlyTheLastWindowStopsTheStreams() {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let store = FleetStore(clients: ["": client])
        let a = UUID(), b = UUID()
        store.open(window: a)
        store.open(window: b)
        store.close(window: a)
        #expect(!client.isStopped, "the other window is still open")
        store.close(window: b)
        #expect(client.isStopped)
    }

    @Test func aWindowClosingTwiceEndsNothing() {
        var windows = WindowSet()
        let a = UUID(), b = UUID()
        windows.open(a)
        windows.open(b)
        let first = windows.close(a), again = windows.close(a), last = windows.close(b)
        #expect(!first)
        #expect(!again)
        #expect(last)
    }

    /// ⌃B x typed in one window closes a pane in that window only.
    @Test func aTileCommandReachesOnlyItsWindow() async throws {
        let heard = Heard()
        var windows: [NSWindow] = []
        for index in 0..<2 {
            windows.append(try await TitleBarHarness.window(TileProbe(index: index, heard: heard), width: 300, height: 200))
        }
        defer { windows.forEach { $0.close() } }
        TileCommand.closePane.post(to: windows[1])
        for window in windows { try await TitleBarHarness.settle(window) }
        #expect(heard.windows == [1])
    }

    /// Typed in a popover or child panel, a command is for the window it
    /// hangs off (review L2).
    @Test func aCommandFromAChildPanelReachesItsWindow() {
        let window = NSWindow(), panel = NSPanel(), other = NSWindow()
        window.addChildWindow(panel, ordered: .above)
        defer { window.removeChildWindow(panel) }
        let box = TileCommand.Box(.closePane, window: panel)
        #expect(box.reaches(window))
        #expect(!box.reaches(other))
    }

    @Test func aCommandForNoWindowReachesNone() {
        let box = TileCommand.Box(.closePane, window: nil)
        #expect(!box.reaches(NSWindow()))
    }

    @MainActor final class Heard {
        var windows: [Int] = []
    }

    struct TileProbe: View {
        let index: Int
        let heard: Heard
        let box = WindowBox()
        var body: some View {
            Text("\(index)")
                .background(WindowReader(box: box))
                .onTileCommand(in: { box.window }) { _ in heard.windows.append(index) }
        }
    }
}
