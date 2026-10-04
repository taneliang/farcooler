import AppKit
import Foundation
import Testing

@testable import Far_Cooler

/// A window's frame, kept and put back (ov-233): read from AppKit's
/// descriptor, and never put where nobody can reach it.
struct WindowFrameTests {
    private static let laptop = CGRect(x: 0, y: 0, width: 1440, height: 875)
    private static let external = CGRect(x: 1440, y: -200, width: 2560, height: 1415)

    @Test("The frame is the descriptor's first four numbers; anything else is none")
    func parsing() {
        #expect(WindowFrame.rect(from: "100 200 900 700 0 0 1440 900") == CGRect(x: 100, y: 200, width: 900, height: 700))
        #expect(WindowFrame.rect(from: "100 200 900") == nil)
        #expect(WindowFrame.rect(from: "") == nil)
        #expect(WindowFrame.rect(from: "100 200 0 700") == nil)
    }

    @Test("A frame with its title bar on a screen stays where it was, on any screen")
    func staysPut() {
        let frame = CGRect(x: 300, y: 100, width: 900, height: 700)
        #expect(WindowFrame.placed(frame, on: [Self.laptop]) == frame)
        let onExternal = CGRect(x: 2000, y: 50, width: 900, height: 700)
        #expect(WindowFrame.placed(onExternal, on: [Self.laptop, Self.external]) == onExternal)
    }

    @Test("A frame left on a display that's gone comes back to the middle of the first screen")
    func offScreen() {
        let gone = CGRect(x: 2000, y: 50, width: 900, height: 700)
        let placed = WindowFrame.placed(gone, on: [Self.laptop])
        #expect(Self.laptop.contains(placed))
        // Centered, within a point either way.
        #expect(abs(placed.midX - Self.laptop.midX) < 1 && abs(placed.midY - Self.laptop.midY) < 1)
        #expect(placed.size == gone.size)
    }

    @Test("A window bigger than the screen it comes back to is cut to fit")
    func tooBig() {
        let huge = CGRect(x: 5000, y: 0, width: 3000, height: 2000)
        let placed = WindowFrame.placed(huge, on: [Self.laptop])
        #expect(Self.laptop.contains(placed))
        #expect(placed.width <= Self.laptop.width && placed.height <= Self.laptop.height)
    }

    @Test("A sliver of title bar on a screen isn't reachable; enough of it is")
    func sliver() {
        // Only 30 points of its width are over the laptop's right edge.
        let sliver = CGRect(x: 1410, y: 100, width: 900, height: 700)
        #expect(WindowFrame.placed(sliver, on: [Self.laptop]) != sliver)
        let enough = CGRect(x: 1300, y: 100, width: 900, height: 700)
        #expect(WindowFrame.placed(enough, on: [Self.laptop]) == enough)
        // Its body on screen with the title bar above the top edge: unreachable too.
        let above = CGRect(x: 300, y: 800, width: 900, height: 700)
        #expect(WindowFrame.placed(above, on: [Self.laptop]) != above)
    }

    @Test("Only SwiftUI's frames for the window's root view are stale")
    func staleKeys() {
        let keys = [
            "NSWindow Frame SwiftUI.ModifiedContent<ContentView, X>-1-AppWindow-1",
            "NSWindow Frame SwiftUI.ModifiedContent<AboutView, X>",
            "NSWindow Frame com.apple.something", "nav.destination.v1",
        ]
        #expect(WindowFrame.staleKeys(in: keys) == [keys[0]])
    }

    @Test("Removing them is done once, and takes nothing else")
    func removal() {
        let suite = "farcooler.test.frames"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        defaults.set("1 2 3 4", forKey: "NSWindow Frame SwiftUI.ModifiedContent<ContentView, A>")
        defaults.set("keep", forKey: "nav.destination.v1")
        WindowFrame.removeStale(from: defaults)
        #expect(defaults.string(forKey: "NSWindow Frame SwiftUI.ModifiedContent<ContentView, A>") == nil)
        #expect(defaults.string(forKey: "nav.destination.v1") == "keep")
        // A later build's key isn't removed behind its back.
        defaults.set("1 2 3 4", forKey: "NSWindow Frame SwiftUI.ModifiedContent<ContentView, B>")
        WindowFrame.removeStale(from: defaults)
        #expect(defaults.string(forKey: "NSWindow Frame SwiftUI.ModifiedContent<ContentView, B>") != nil)
    }
}
