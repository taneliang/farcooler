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
        words.nowDoing = "User test issues and the review queue"
        let window = try await TitleBarHarness.window(
            TitleBarHarness.Root(words: words, content: Color.clear), width: 1400, height: 200)
        defer { window.close() }
        window.appearance = NSAppearance(named: variant == "dark" ? .darkAqua : .aqua)
        try await TitleBarHarness.settle(window)
        if let rep = RealWindowCaptures.windowImage(window) {
            try rep.representation(using: .png, properties: [:])?
                .write(to: URL(fileURLWithPath: out).appendingPathComponent("title-orchestrator-\(variant).png"))
        }
    }
}
