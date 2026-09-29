import Foundation
import Testing

@testable import Far_Cooler

/// The one-time tip that workspaces are in the sidebar (spec §9).
struct WorkspacesTipTests {
    /// Shown on the first launch, and on none after it's dismissed: the
    /// dismissal is kept under `tips.workspaces`.
    @Test("The tip shows once")
    func theTipShowsOnce() {
        let name = "fc-tip-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        #expect(WorkspacesTip.shouldShow(defaults))
        #expect(WorkspacesTip.shouldShow(defaults), "asking dismissed it")
        WorkspacesTip.dismiss(defaults)
        #expect(!WorkspacesTip.shouldShow(defaults))
        #expect(defaults.bool(forKey: "tips.workspaces"))
        #expect(WorkspacesTip.title == "Workspaces are now in the sidebar.")
    }
}
