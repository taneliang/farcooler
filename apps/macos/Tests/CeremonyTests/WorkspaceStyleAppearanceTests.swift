import AppKit
import Testing

@testable import Far_Cooler

/// ov-81 P2: the board's surfaces must follow the appearance, not the one that
/// was current when a view's body last ran.
@MainActor
struct WorkspaceStyleAppearanceTests {
    private func brightness(_ color: NSColor, _ name: NSAppearance.Name) -> CGFloat {
        var result: CGFloat = 0
        NSAppearance(named: name)!.performAsCurrentDrawingAppearance {
            result = (color.usingColorSpace(.sRGB) ?? color).brightnessComponent
        }
        return result
    }

    @Test("Every themed surface resolves light under Aqua and dark under Dark Aqua")
    func surfacesFollowTheAppearance() {
        // Read ONCE, as a view body does, then resolved under each appearance.
        let surfaces: [(String, NSColor)] = [
            ("canvas", WorkspaceStyle.canvasNS), ("document", WorkspaceStyle.documentNS),
            ("paneChrome", WorkspaceStyle.paneChromeNS),
        ]
        for (name, color) in surfaces {
            let light = brightness(color, .aqua)
            let dark = brightness(color, .darkAqua)
            #expect(light > 0.6, "\(name) in light: \(light)")
            #expect(dark < 0.4, "\(name) in dark: \(dark)")
        }
    }
}
