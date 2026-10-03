import Foundation
import Testing

@testable import Far_Cooler

/// The one-time tip that the workspace has a navigator (ov-92).
struct WorkspacesTipTests {
    /// Shown on the first launch, and on none after it's dismissed: the
    /// dismissal is kept under `tips.workspaceNavigator` (ov-92). The old
    /// tips' dismissals (`tips.workspaces`, `tips.tasksBesideBoard`,
    /// `tips.orchestratorFillsWorkspace`) don't hide it: it says something
    /// new.
    @Test("The tip shows once")
    func theTipShowsOnce() {
        let name = "fc-tip-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        defaults.set(true, forKey: "tips.workspaces")
        defaults.set(true, forKey: "tips.tasksBesideBoard")
        defaults.set(true, forKey: "tips.orchestratorFillsWorkspace")
        #expect(WorkspacesTip.shouldShow(defaults))
        #expect(WorkspacesTip.shouldShow(defaults), "asking dismissed it")
        WorkspacesTip.dismiss(defaults)
        #expect(!WorkspacesTip.shouldShow(defaults))
        #expect(defaults.bool(forKey: "tips.workspaceNavigator"))
        #expect(WorkspacesTip.title == "Your workspace has a navigator.")
    }
}
