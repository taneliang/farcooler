import AppKit
import Testing

@testable import Far_Cooler

/// The switcher's bezel (ov-91): standard hover and press highlighting.
@MainActor
struct WorkspaceSwitcherButtonTests {
    @Test("A toolbar-style button that shows its border only under the pointer")
    func hoverHighlight() {
        let button = NSButton()
        WorkspaceSwitcherButton.configure(button)
        #expect(button.isBordered)
        #expect(button.bezelStyle == .recessed)
        #expect(button.showsBorderOnlyWhileMouseInside)
    }
}
