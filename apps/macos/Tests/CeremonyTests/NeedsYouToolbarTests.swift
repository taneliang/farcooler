import Testing

@testable import Far_Cooler

/// What the title bar's Needs You button says and how it is tinted (ov-91).
struct NeedsYouToolbarTests {
    @Test("The tooltip names the count, and says nothing of it when nothing waits")
    func tooltipNamesTheCount() {
        #expect(NeedsYouToolbar.tooltip(count: 0) == "Needs You")
        #expect(NeedsYouToolbar.tooltip(count: 1) == "Needs You (1)")
        #expect(NeedsYouToolbar.tooltip(count: 12) == "Needs You (12)")
    }

    @Test("It is tinted with the accent only while something waits")
    func tintFollowsTheCount() {
        #expect(!NeedsYouToolbar.isTinted(count: 0))
        #expect(NeedsYouToolbar.isTinted(count: 1))
    }

    @Test("The badge shows nothing at zero and caps at 99+")
    func badgeText() {
        #expect(NeedsYouToolbar.badge(count: 0) == nil)
        #expect(NeedsYouToolbar.badge(count: 7) == "7")
        #expect(NeedsYouToolbar.badge(count: 120) == "99+")
    }

    @Test("VoiceOver hears the count in words")
    func accessibility() {
        #expect(NeedsYouToolbar.accessibilityLabel(count: 0) == "Needs You, nothing waiting")
        #expect(NeedsYouToolbar.accessibilityLabel(count: 1) == "Needs You, 1 item")
        #expect(NeedsYouToolbar.accessibilityLabel(count: 3) == "Needs You, 3 items")
    }
}
