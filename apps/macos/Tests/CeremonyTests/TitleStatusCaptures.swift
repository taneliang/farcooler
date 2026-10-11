import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The title bar's center at the minimum, medium and wide windows, light and
/// dark (ov-469), as PNGs for a person to read. Opt-in, never in CI: enabled
/// by `FARCOOLER_TITLEBAR_CAPTURE_OUT`, a directory.
@MainActor
@Suite(.serialized)
struct TitleStatusCaptures {
    typealias Harness = TitleBarHarness

    @Test(.enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_TITLEBAR_CAPTURE_OUT"] != nil))
    func capture() async throws {
        let out = URL(fileURLWithPath: try #require(ProcessInfo.processInfo.environment["FARCOOLER_TITLEBAR_CAPTURE_OUT"]))
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let store = await TitleStatusTests.store()
        let cases: [(String, CGFloat, String, Int)] = [
            ("minimum", MainWindowChrome.minimumWidth, "Delegating ov-468", 4),
            ("medium", 900, "Delegating ov-468", 4),
            ("wide", 1400, "Delegating ov-468", 4),
            ("medium-long", 900, "Reading ov-192’s jump bar diff, then the integration report for integ-4", 142),
        ]
        for (name, width, doing, needYou) in cases {
            for (look, appearance) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
                let words = Harness.Words()
                words.nowDoing = doing
                words.needYou = needYou
                let window = try await Harness.window(
                    Harness.Root(words: words, board: store, content: Color.clear), width: width, height: 120,
                    beforeShowing: {})
                window.appearance = NSAppearance(named: appearance)
                try await Harness.settle(window)
                let view = try #require(window.contentView?.superview)
                let rep = try #require(view.lookBitmap(scale: 2))
                let png = try #require(rep.representation(using: .png, properties: [:]))
                try png.write(to: out.appendingPathComponent("titlebar-\(name)-\(Int(width))-\(look).png"))
                try await TitleBarGapTests.close(window)
            }
        }
    }
}
