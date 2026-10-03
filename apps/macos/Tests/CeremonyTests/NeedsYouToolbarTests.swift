import Testing

@testable import Far_Cooler

/// What the title bar's Needs You item says and how it's colored (ov-91,
/// quieted in ov-105).
struct NeedsYouToolbarTests {
    @Test("The tooltip names the count, and says nothing of it when nothing waits")
    func tooltipNamesTheCount() {
        #expect(NeedsYouToolbar.tooltip(count: 0) == "Needs You")
        #expect(NeedsYouToolbar.tooltip(count: 1) == "Needs You (1)")
        #expect(NeedsYouToolbar.tooltip(count: 12) == "Needs You (12)")
    }

    @Test("The count is text beside the tray: none at zero, capped at 99+")
    func countText() {
        #expect(NeedsYouToolbar.countText(count: 0) == nil)
        #expect(NeedsYouToolbar.countText(count: 3) == "3")
        #expect(NeedsYouToolbar.countText(count: 120) == "99+")
    }

    @Test("Only the count wears the accent, and only while something waits")
    func accentFollowsTheCount() {
        #expect(!NeedsYouToolbar.countIsAccent(count: 0))
        #expect(NeedsYouToolbar.countIsAccent(count: 3))
    }

    @Test("VoiceOver hears how many are waiting")
    func accessibility() {
        #expect(NeedsYouToolbar.accessibilityLabel(count: 0) == "Needs You, nothing waiting")
        #expect(NeedsYouToolbar.accessibilityLabel(count: 1) == "Needs You, 1 waiting")
        #expect(NeedsYouToolbar.accessibilityLabel(count: 3) == "Needs You, 3 waiting")
    }
}
