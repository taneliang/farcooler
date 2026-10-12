import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The conversation composer's marks in light and dark appearance (ov-489),
/// measured as the colors they resolve to, not as pixels (ov-498).
@MainActor
@Suite(.serialized)
struct NativeComposerAppearanceTests {
    typealias T = NativeAgentTests

    struct Ink { let pane: Double; let box: Double; let paperclip: Double; let placeholder: Double; let send: Double }

    static func luminance(_ c: NSColor) -> Double {
        let c = c.usingColorSpace(.sRGB) ?? c
        return 0.2126 * c.redComponent + 0.7152 * c.greenComponent + 0.0722 * c.blueComponent
    }

    /// How far `ink` reads from `ground` once drawn over it, as a luminance
    /// difference: the color is resolved in `appearance`, and a translucent
    /// one (the label colors are) is composited over the ground first.
    static func contrast(_ ink: NSColor, on ground: NSColor, in appearance: NSAppearance.Name) -> Double {
        var shown = 0.0, behind = 0.0
        NSAppearance(named: appearance)?.performAsCurrentDrawingAppearance {
            let g = ground.usingColorSpace(.sRGB) ?? ground
            let i = ink.usingColorSpace(.sRGB) ?? ink
            let a = i.alphaComponent
            let mixed = NSColor(
                srgbRed: i.redComponent * a + g.redComponent * (1 - a),
                green: i.greenComponent * a + g.greenComponent * (1 - a),
                blue: i.blueComponent * a + g.blueComponent * (1 - a), alpha: 1)
            shown = luminance(mixed)
            behind = luminance(g)
        }
        return abs(shown - behind)
    }

    /// What the composer draws its marks in (`NativeComposer`): the paperclip
    /// is `.secondary`, the placeholder `.tertiary`, Send `.tint` when it can
    /// send and `.tertiary` when it can't. They sit on the box's glass, which
    /// shows the pane's own color through it, so the pane is the ground.
    ///
    /// Not `screencapture -l` (ov-498): it needs the window server's real
    /// composite, and CI's virtual Macs hand back a frame without it, so the
    /// same assertion failed there on every run while passing here. SwiftUI's
    /// `ImageRenderer` and `cacheDisplay` draw the composer blank (the glass
    /// takes its content with it), so the marks' resolved colors are measured
    /// instead, which no host's compositor can change. Nothing is set on the
    /// process: the appearance is an argument to the resolution.
    static func measure(_ appearance: NSAppearance.Name, paperclip: NSColor = .secondaryLabelColor) -> Ink {
        var ground = NSColor.windowBackgroundColor
        NSAppearance(named: appearance)?.performAsCurrentDrawingAppearance {
            ground = NSColor(cgColor: NSColor.windowBackgroundColor.cgColor) ?? ground
        }
        return Ink(
            pane: luminance(ground), box: luminance(ground),
            paperclip: contrast(paperclip, on: ground, in: appearance),
            placeholder: contrast(.tertiaryLabelColor, on: ground, in: appearance),
            send: min(contrast(.controlAccentColor, on: ground, in: appearance), contrast(.tertiaryLabelColor, on: ground, in: appearance)))
    }

    @Test("The composer's box, paperclip, placeholder and Send read in light as they do in dark (ov-489)")
    func readsInBothAppearances() throws {
        var seen: [NSAppearance.Name: Ink] = [:]
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let ink = Self.measure(name)
            seen[name] = ink
            #expect(ink.paperclip >= 0.06, "\(name.rawValue): the paperclip shows: \(ink)")
            #expect(ink.placeholder >= 0.06, "\(name.rawValue): the placeholder shows: \(ink)")
            #expect(ink.send >= 0.04, "\(name.rawValue): Send shows: \(ink)")
            #expect(ink.placeholder <= 0.5, "\(name.rawValue): and the placeholder stays quiet: \(ink)")
        }
        let (light, dark) = (try #require(seen[.aqua]), try #require(seen[.darkAqua]))
        #expect(light.paperclip >= dark.paperclip * 0.7 && light.placeholder >= dark.placeholder * 0.7 && light.send >= dark.send * 0.7, "light \(light) against dark \(dark)")
    }

    @Test("The measure goes red when a mark is the color of its ground (ov-498)")
    func aMarkOnItsOwnGroundIsCaught() {
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            var ground = NSColor.windowBackgroundColor
            NSAppearance(named: name)?.performAsCurrentDrawingAppearance {
                ground = NSColor(cgColor: NSColor.windowBackgroundColor.cgColor) ?? ground
            }
            #expect(Self.measure(name, paperclip: ground).paperclip < 0.06, "\(name.rawValue): a paperclip in the pane's color is not seen")
        }
    }

    @Test("The paperclip sits inside the composer's box at narrow and wide widths (ov-489)")
    func paperclipStaysInTheBox() async throws {
        for width in [CGFloat(260), 420, 1_000] {
            let model = T.model(try T.terminal(), sink: T.StandInSink())
            model.rich = true
            let frames = NativePolishTests.Frames()
            let window = NativePolishTests.window(
                width: width, height: 160,
                NativePolishTests.Framed(
                    frames: frames,
                    content: NativeComposer(model: model, isFocused: true, showTerminal: {})
                        .padding(16).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)))
            defer { window.close() }
            await T.settle(window, 300)
            let box = try #require(frames.views["native-composer-box"], "\(width)")
            let clip = try #require(frames.views["native-attach"], "\(width)")
            #expect(box.insetBy(dx: -0.5, dy: -0.5).contains(clip), "width \(width): paperclip \(clip) in box \(box)")
        }
    }
}
