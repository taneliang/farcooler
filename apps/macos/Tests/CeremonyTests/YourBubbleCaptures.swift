import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the person's messages as the site's blue
/// bubble (ov-610), a short one and a long one folded with Show More, beside
/// the agent's reply, light and dark. The production
/// `NativeAgentView` over a pane model whose rows come through its own
/// ledger, in a real titled window off every screen, taken with
/// `screencapture -l` of that window. Sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct YourBubbleCaptures {
    @Test func shortAndLongMessages() async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let long = String(repeating: "Fold this where it wraps past six lines, as a long message does. ", count: 9)
        // Not the high-contrast appearances: SwiftUI's contrast on the Mac
        // follows the system's Increase Contrast, not the window's
        // appearance, so those came out the same as these (review 1).
        for (name, appearance) in RealWindowCaptures.variants where !name.contains("contrast") {
            let model = NativeAgentTests.model(try NativeAgentTests.terminal(), sink: NativeAgentTests.StandInSink())
            model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([
                NativeAgentTests.row(0, "turn:p1", ["Turn": [
                    "prompt": "Why does the parser test fail?", "origin": "Typed", "started_ms": 1, "ended_ms": 9_000,
                    "duration_ms": 9_000, "outcome": "Finished", "background_running": 0, "activity": NSNull(),
                ]]),
                NativeAgentTests.row(1, "prose:1", ["Prose": [
                    "text": "The empty input reaches `parse` before the length check, so it indexes past the end.",
                    "conclusion": true, "at_ms": 9_000,
                ]]),
                NativeAgentTests.row(2, "turn:p2", ["Turn": [
                    "prompt": long, "origin": "Typed", "started_ms": 10_000, "ended_ms": 19_000,
                    "duration_ms": 9_000, "outcome": "Finished", "background_running": 0, "activity": "Idle",
                ]]),
            ])))
            let window = NativeAgentTests.window(NativeAgentView(model: model, isFocused: true, showTerminal: {}))
            window.appearance = NSAppearance(named: appearance)
            await NativeAgentTests.settle(window, 600)
            let image = try #require(RealWindowCaptures.windowImage(window), "screencapture -l isn't allowed here")
            try #require(image.representation(using: .png, properties: [:]))
                .write(to: URL(fileURLWithPath: out).appendingPathComponent("ov-610-after-mac-\(name).png"))
            window.close()
        }
    }
}
