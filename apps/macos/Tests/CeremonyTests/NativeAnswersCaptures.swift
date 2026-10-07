import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): a held question, plan and permission in
/// the native view (ov-370), in the four appearances. The production
/// `NativeAgentView` over a pane model whose rows come through its own
/// ledger, in a real titled window off every screen, taken with
/// `screencapture -l` of that window. Sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct NativeAnswersCaptures {
    static func turn(_ ord: Int, _ prompt: String) -> [String: Any] {
        NativeAgentTests.row(ord, "turn:p\(ord)", ["Turn": [
            "prompt": prompt, "origin": "Typed", "started_ms": Int64(Date().timeIntervalSince1970 * 1000) - 42_000, "ended_ms": NSNull(), "duration_ms": NSNull(),
            "outcome": NSNull(), "background_running": 0, "activity": "Waiting",
        ]])
    }

    @Test(arguments: ["question", "plan", "permission"])
    func heldAsk(_ which: String) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let ask: [String: Any] = switch which {
        case "question":
            NativeAnswersTests.question().merging(["questions": [[
                "question": "Which color should the button be?", "header": "Color", "multi_select": false,
                "options": [["label": "Red", "description": "Warm and loud"], ["label": "Blue", "description": "Calm and quiet"]],
            ]], "text": "Which color should the button be?"]) { $1 }
        case "plan":
            NativeAnswersTests.plan().merging(["plan": "# Plan\n\n1. Make the button blue.\n2. Add a test for its color.\n3. Ship it."]) { $1 }
        default:
            NativeAnswersTests.permission().merging(["text": "Bash touch spike-made-this.txt"]) { $1 }
        }
        for (name, appearance) in RealWindowCaptures.variants {
            let model = NativeAgentTests.model(try NativeAgentTests.terminal())
            model.answers = NativeAnswersTests.StandInAnswers()
            model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([
                Self.turn(0, "Make the sign-up button stand out."),
                NativeAgentTests.row(1, "ask:toolu_1", ["Ask": ask]),
            ])))
            let window = NativeAgentTests.window(NativeAgentView(model: model, isFocused: true, showTerminal: {}))
            window.appearance = NSAppearance(named: appearance)
            await NativeAgentTests.settle(window, 600)
            let image = try #require(RealWindowCaptures.windowImage(window), "screencapture -l isn't allowed here")
            try #require(image.representation(using: .png, properties: [:]))
                .write(to: URL(fileURLWithPath: out).appendingPathComponent("held-\(which)-\(name).png"))
            window.close()
        }
    }
}
