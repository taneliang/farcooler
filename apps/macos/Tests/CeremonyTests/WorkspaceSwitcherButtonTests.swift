import Testing

@testable import Far_Cooler

/// The switcher in the toolbar's leading group (ov-105): VoiceOver hears the
/// workspace, then its repository.
struct WorkspaceSwitcherButtonTests {
    @Test("VoiceOver reads the workspace and its repository")
    func accessibilityLabel() {
        #expect(WorkspaceSwitcherButton.accessibilityLabel(title: "Main", repository: "overnight")
            == "Workspace: Main, overnight")
        #expect(WorkspaceSwitcherButton.accessibilityLabel(title: "Needs You", repository: "") == "Workspace: Needs You")
    }
}
