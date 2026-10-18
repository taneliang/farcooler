import AppKit
import QuartzCore
import TestWindows
import Testing

/// What `TestWindow` promises the suite and the person running it (ov-473).
@MainActor
struct TestWindowTests {
    final class Ticks: NSObject {
        var count = 0
        @objc func tick(_ link: CADisplayLink) { count += 1 }
    }

    static func window() -> NSWindow {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 200, height: 200), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        return window
    }

    /// The animated scrolls of ov-429 land only on display ticks, and a
    /// window off every screen gets none.
    @Test("A test window still gets display ticks")
    func ticks() async throws {
        let window = Self.window()
        TestWindow.show(window)
        defer { window.close() }
        let ticks = Ticks()
        let link = try #require(window.contentView).displayLink(target: ticks, selector: #selector(Ticks.tick(_:)))
        link.add(to: .main, forMode: .common)
        defer { link.invalidate() }
        // Poll, not count over a fixed window: CI's virtual Macs tick slowly
        // and unevenly (ov-500). 5 ticks within 30 s is far from none.
        try await NativeDefaultOnTests.until(120) { ticks.count >= 5 }
        #expect(ticks.count >= 5, "\(ticks.count) display ticks in 30 seconds")
    }

    @Test("A test window sits behind every app's windows and lets clicks through")
    func behind() {
        let window = Self.window()
        TestWindow.show(window)
        defer { window.close() }
        #expect(window.isVisible)
        #expect(window.level < .normal)
        #expect(window.ignoresMouseEvents)
    }

    /// The person's app stays frontmost: the process can't be activated,
    /// even when a test asks.
    @Test("The test process can never come to the front")
    func neverActive() async throws {
        let window = Self.window()
        TestWindow.show(window)
        defer { window.close() }
        try #require(NSApp.activationPolicy() == .prohibited, "the next line would bring the run to the front")
        NSApp.activate()  // window-exempt: the request this test proves is refused
        try await Task.sleep(for: .milliseconds(200))
        #expect(!NSApp.isActive)
        #expect(!window.isKeyWindow)
    }
}
