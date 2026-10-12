import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The conversation composer in light and dark appearance (ov-489), as the
/// window server composites it (`screencapture -l`), not as `cacheDisplay`
/// draws it, which leaves the glass and the SwiftUI text out. The window is
/// the test process's: off screen and never the active one, which this
/// process cannot become, so both appearances are measured in that same state.
@MainActor
@Suite(.serialized)
struct NativeComposerAppearanceTests {
    typealias T = NativeAgentTests

    struct Ink { let pane: Double; let box: Double; let paperclip: Double; let placeholder: Double; let send: Double }

    static func luminance(_ c: NSColor) -> Double {
        let c = c.usingColorSpace(.sRGB) ?? c
        return 0.2126 * c.redComponent + 0.7152 * c.greenComponent + 0.0722 * c.blueComponent
    }

    /// How far from the box's own background the ink in `region` goes, as a luminance difference.
    static func contrast(_ rep: NSBitmapImageRep, _ region: CGRect, _ background: Double) -> Double {
        var best = 0.0
        for y in Int(region.minY)..<Int(region.maxY) {
            for x in Int(region.minX)..<Int(region.maxX) {
                guard let c = rep.colorAt(x: x, y: y) else { continue }
                best = max(best, abs(luminance(c) - background))
            }
        }
        return best
    }

    static func measure(_ appearance: NSAppearance.Name) async throws -> Ink {
        let model = T.model(try T.terminal(), sink: T.StandInSink())
        model.rich = true
        let frames = NativePolishTests.Frames()
        let view = NativePolishTests.Framed(
            frames: frames,
            content: NativeComposer(model: model, isFocused: true, showTerminal: {})
                .padding(16)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .background(Color(nsColor: .windowBackgroundColor))
                )
        let window = NativePolishTests.window(width: 520, height: 120, view)
        defer { window.close() }
        window.appearance = NSAppearance(named: appearance)
        await T.settle(window, 400)
        let content = try #require(window.contentView)
        let rep = try #require(RealWindowCaptures.windowImage(window), "screencapture -l isn't allowed here")
        let scale = Double(rep.pixelsWide) / Double(content.bounds.width)
        let top = Double(rep.pixelsHigh) - Double(content.bounds.height) * scale
        func px(_ id: String) -> CGRect {
            let f = frames.views[id] ?? .zero
            return CGRect(x: f.minX * scale, y: top + f.minY * scale, width: f.width * scale, height: f.height * scale)
        }
        let box = px("native-composer-box")
        let backdrop = luminance(rep.colorAt(x: Int(box.midX), y: Int(box.midY)) ?? .black)
        let pane = luminance(rep.colorAt(x: Int(box.midX), y: Int(box.minY) - Int(8 * scale)) ?? .black)
        let clip = px("native-attach")
        let field = px("native-composer")
        return Ink(
            pane: pane, box: backdrop,
            paperclip: contrast(rep, clip, backdrop),
            placeholder: contrast(rep, CGRect(x: field.minX, y: field.minY, width: 100 * scale, height: field.height), backdrop),
            send: contrast(rep, CGRect(x: box.maxX - 20 * scale, y: box.minY + 4, width: 14 * scale, height: box.height - 8), backdrop))
    }

    @Test("The composer's box, paperclip, placeholder and Send read in light as they do in dark (ov-489)")
    func readsInBothAppearances() async throws {
        var seen: [NSAppearance.Name: Ink] = [:]
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let ink = try await Self.measure(name)
            seen[name] = ink
            #expect(abs(ink.box - ink.pane) >= 0.02, "\(name.rawValue): the box is a visible surface on its pane: \(ink)")
            #expect(ink.paperclip >= 0.06, "\(name.rawValue): the paperclip shows: \(ink)")
            #expect(ink.placeholder >= 0.06, "\(name.rawValue): the placeholder shows: \(ink)")
            #expect(ink.send >= 0.04, "\(name.rawValue): Send shows: \(ink)")
            #expect(ink.placeholder <= 0.5, "\(name.rawValue): and the placeholder stays quiet: \(ink)")
        }
        let (light, dark) = (try #require(seen[.aqua]), try #require(seen[.darkAqua]))
        #expect(light.paperclip >= dark.paperclip * 0.7 && light.placeholder >= dark.placeholder * 0.7 && light.send >= dark.send * 0.7, "light \(light) against dark \(dark)")
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
