import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// ov-81 P7: a prompt must not read as a typed value. Measured on a rendered
/// grouped form, the way the reviewer saw it: the strongest text pixel, as a
/// share of the strongest one a typed value draws.
@MainActor
struct FieldPromptTests {
    /// How far the most contrasting pixel is from the background, 0 to 1.
    private func contrast<V: View>(_ view: V, dark: Bool) -> Double {
        let host = NSHostingView(rootView: view.frame(width: 400, height: 80))
        host.frame = NSRect(x: 0, y: 0, width: 400, height: 80)
        let window = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        window.contentView = host
        host.layoutSubtreeIfNeeded()
        host.display()
        guard let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { return -1 }
        host.cacheDisplay(in: host.bounds, to: rep)
        var best = 0.0
        var worst = 1.0
        for y in 0..<rep.pixelsHigh {
            for x in 0..<rep.pixelsWide {
                let b = Double(rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB)?.brightnessComponent ?? 0)
                best = max(best, b)
                worst = min(worst, b)
            }
        }
        return dark ? best : 1 - worst
    }

    @Test("A field prompt is clearly dimmer than a typed value", arguments: [true, false])
    func promptIsDimmerThanAValue(dark: Bool) {
        let value = contrast(
            Form { TextField("", text: .constant("user@host")) }.formStyle(.grouped), dark: dark)
        let prompt = contrast(
            Form { TextField("", text: .constant(""), prompt: .fieldPrompt("user@host")) }
                .formStyle(.grouped), dark: dark)
        #expect(prompt < value * 0.75, "prompt \(prompt) vs value \(value), dark \(dark)")
    }
}
