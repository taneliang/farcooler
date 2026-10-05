import AppKit
import Testing

@testable import Far_Cooler

/// What a bare Esc puts away before the window's other keys hear it (ov-298).
struct PlanPeekEscapeTests {
    /// Train 1004r, P3: Esc reached the terminal under the peek and
    /// interrupted the orchestrator.
    @Test("A bare Esc puts the peek away first, then a floated navigator, and nothing else")
    func escapePutsAwayFirst() {
        #expect(OverlayEscape.puts(keyCode: 53, modifiers: [], peeking: true, floating: true) == .peek)
        #expect(OverlayEscape.puts(keyCode: 53, modifiers: [], peeking: false, floating: true) == .navigator)
        #expect(OverlayEscape.puts(keyCode: 53, modifiers: [], peeking: false, floating: false) == nil)
        #expect(OverlayEscape.puts(keyCode: 53, modifiers: .shift, peeking: true, floating: false) == nil)
        #expect(OverlayEscape.puts(keyCode: 36, modifiers: [], peeking: true, floating: false) == nil)
    }
}
