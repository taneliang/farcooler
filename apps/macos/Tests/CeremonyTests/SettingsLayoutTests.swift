import CoreGraphics
import Testing

@testable import Far_Cooler

/// Settings opens on General, and no sheet it opens is larger than it.
struct SettingsLayoutTests {
    /// Behavior and Startup became sections of General. A stored tab that
    /// names either must land somewhere real, and every other value is left
    /// alone.
    @Test("Behavior and Startup open General")
    func retiredTabsOpenGeneral() {
        #expect(SettingsTab.normalized("behavior") == SettingsTab.general)
        #expect(SettingsTab.normalized("startup") == SettingsTab.general)
        #expect(SettingsTab.normalized("machines") == "machines")
        #expect(SettingsTab.normalized("terminal") == "terminal")
        #expect(SettingsTab.general == "general")
    }

    /// A sheet larger than its host hangs off it, so each step is no larger
    /// than the one under it: Settings, the runner sheet, then the editors.
    @Test("Sheets are never larger than the Settings window")
    func sheetsFitTheirHost() {
        let window = SettingsSheetSize.window
        let runner = SettingsSheetSize.runner
        let editor = SettingsSheetSize.editor
        #expect(runner.width <= window.width && runner.height <= window.height)
        #expect(editor.width <= runner.width && editor.height <= runner.height)
    }
}
