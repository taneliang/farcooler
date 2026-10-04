import AppKit
import Testing

@testable import Far_Cooler

/// ⌘W with Settings key closes Settings, and leaves the terminal behind it
/// running (ov-265).
///
/// It did neither before: Close Terminal held ⌘W, SwiftUI took the chord off
/// File ▸ Close for it, and with Settings key the item was disabled, which in
/// AppKit swallows the chord rather than passing it on.
@MainActor
struct CloseCommandTests {
    /// Every close command posted while `body` runs.
    private static func posted(_ body: () -> Void) -> [String] {
        var heard: [String] = []
        let token = NotificationCenter.default.addObserver(
            forName: AppCommand.notification, object: nil, queue: nil
        ) { note in
            if let raw = note.object as? String, raw == AppCommand.closeTerminal.rawValue { heard.append(raw) }
        }
        body()
        NotificationCenter.default.removeObserver(token)
        return heard
    }

    private static func window() -> NSWindow {
        let window = NSWindow(
            contentRect: NSRect(x: -6000, y: -6000, width: 300, height: 200), styleMask: [.titled, .closable],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.orderFrontRegardless()
        return window
    }

    @Test("With Settings key, ⌘W closes Settings and closes no terminal")
    func settingsKeyClosesSettings() {
        let settings = Self.window()
        defer { settings.close() }
        let close = CloseCommand.at(nil)
        #expect(close == .window && close.title == "Close Window")
        let heard = Self.posted { close.perform(keyWindow: settings) }
        #expect(!settings.isVisible, "Settings stayed open")
        #expect(heard.isEmpty, "the terminal behind it was closed")
    }

    @Test("With the main window key, ⌘W closes the selected terminal and not the window")
    func mainWindowKeyClosesTheTerminal() {
        let main = Self.window()
        defer { main.close() }
        let close = CloseCommand.at(MainWindowFocus(overlayOpen: false))
        #expect(close == .terminal && close.title == "Close Terminal")
        let heard = Self.posted { close.perform(keyWindow: main) }
        #expect(heard == [AppCommand.closeTerminal.rawValue])
        #expect(main.isVisible)
    }

    /// Why ⌘W can't be disabled for Settings: AppKit's own key-equivalent
    /// match, on a File menu shaped as SwiftUI built it, a disabled ⌘W item
    /// before an enabled Close.
    @Test("A disabled ⌘W item swallows the chord instead of passing it to the next")
    func aDisabledItemSwallowsTheChord() throws {
        final class Target: NSObject {
            var hits: [String] = []
            @objc func terminal(_ sender: Any?) { hits.append("terminal") }
            @objc func close(_ sender: Any?) { hits.append("close") }
        }
        let target = Target()
        let bar = NSMenu(), file = NSMenu(title: "File")
        bar.autoenablesItems = false
        file.autoenablesItems = false
        let top = NSMenuItem(title: "File", action: nil, keyEquivalent: "")
        top.submenu = file
        bar.addItem(top)
        let terminal = NSMenuItem(title: "Close Terminal", action: #selector(Target.terminal), keyEquivalent: "w")
        terminal.target = target
        terminal.isEnabled = false
        let close = NSMenuItem(title: "Close", action: #selector(Target.close), keyEquivalent: "w")
        close.target = target
        file.addItem(terminal)
        file.addItem(close)
        let chord = try #require(
            NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0, windowNumber: 0, context: nil,
                characters: "w", charactersIgnoringModifiers: "w", isARepeat: false, keyCode: 13))
        #expect(bar.performKeyEquivalent(with: chord), "the menu took the chord")
        #expect(target.hits.isEmpty, "a disabled item passed ⌘W on: \(target.hits)")
    }

    @Test("Close All closes every window that can be closed")
    func closeAll() {
        let one = Self.window(), two = Self.window()
        defer {
            one.close()
            two.close()
        }
        CloseCommand.closeAll([one, two])
        #expect(!one.isVisible && !two.isVisible)
    }
}
