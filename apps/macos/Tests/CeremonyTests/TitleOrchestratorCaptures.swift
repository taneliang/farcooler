import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the real title bar, light and dark (ov-320).
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct TitleOrchestratorCaptures {
    @Test(arguments: (ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_VARIANT"] ?? "light,dark").split(separator: ",").map(String.init))
    func titleBar(variant: String) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        let words = TitleBarHarness.Words()
        // A line as the agent's screen gives it, furniture and all, through
        // the sanitizer the row uses (ov-329).
        words.nowDoing = NowDoingText.clean("✻ Reviewing the gesture fix and the review queue… (esc to interrupt)")
        // `FARCOOLER_CAPTURE_WIDTH`: 900 draws the medium form, 1790 the wide.
        let width = Double(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_WIDTH"] ?? "") ?? 1400
        let window = try await TitleBarHarness.window(
            TitleBarHarness.Root(words: words, content: Color.clear), width: width, height: 200)
        defer { window.close() }
        window.appearance = NSAppearance(named: variant == "dark" ? .darkAqua : .aqua)
        try await TitleBarHarness.settle(window)
        if let rep = RealWindowCaptures.windowImage(window) {
            try rep.representation(using: .png, properties: [:])?
                .write(to: URL(fileURLWithPath: out).appendingPathComponent("title-orchestrator-\(Int(width))-\(variant).png"))
        }
    }
}
