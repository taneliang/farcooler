import Foundation
import Testing

@testable import Far_Cooler

/// The one-time tip that workspaces are in the sidebar (spec §9).
struct WorkspacesTipTests {
    /// Shown on the first launch, and on none after it's dismissed: the
    /// dismissal is kept under `tips.orchestratorFillsWorkspace`. The old
    /// tips' dismissals (`tips.workspaces`, `tips.tasksBesideBoard`) don't
    /// hide it: it says something new.
    @Test("The tip shows once")
    func theTipShowsOnce() {
        let name = "fc-tip-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        defaults.set(true, forKey: "tips.workspaces")
        defaults.set(true, forKey: "tips.tasksBesideBoard")
        #expect(WorkspacesTip.shouldShow(defaults))
        #expect(WorkspacesTip.shouldShow(defaults), "asking dismissed it")
        WorkspacesTip.dismiss(defaults)
        #expect(!WorkspacesTip.shouldShow(defaults))
        #expect(defaults.bool(forKey: "tips.orchestratorFillsWorkspace"))
        #expect(WorkspacesTip.title == "The orchestrator now fills the workspace.")
    }
}
