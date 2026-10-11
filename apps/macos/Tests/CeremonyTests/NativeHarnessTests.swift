import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The turns claude writes for itself (ov-474), drawn in a real offscreen
/// window from the page the client core writes out of a redacted real
/// transcript (`test/fixtures/agent-rows-harness.json`): a subagent's report
/// folds under its agent's name, a task notice is one muted line, a reminder
/// is a closed System note that opens. The words are AgentKit's and tested
/// there (`HarnessRowsTests`); the rows are the projector's (`harness_tests.rs`).
@MainActor
@Suite(.serialized)
struct NativeHarnessTests {
    typealias T = NativeAgentTests

    /// The fixture's rows, found from this file:
    /// …/apps/macos/Tests/CeremonyTests/<this file>.
    static func rows() throws -> [[String: Any]] {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { url.deleteLastPathComponent() }
        let data = try Data(contentsOf: url.appendingPathComponent("test/fixtures/agent-rows-harness.json"))
        let page = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        return try #require(page["rows"] as? [[String: Any]])
    }

    @Test("A report, a task notice and a reminder each have their own row, and the reminder opens")
    func theirOwnRows() async throws {
        let model = T.model(try T.terminal())
        let frames = NativePolishTests.Frames()
        let window = NativePolishTests.window(
            width: 720, height: 1_400,
            NativePolishTests.Framed(frames: frames, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
        defer { window.close() }
        model.store.apply(try await model.store.ledger.page(T.page(try Self.rows())))
        await T.settle(window, 300)
        var seen: Set<String> { Set(frames.views.keys) }

        #expect(seen.contains("native-harness-report"), "a hand-back is a report")
        #expect(seen.contains("native-harness-status"), "a task notice is a status line")
        #expect(seen.contains("native-harness-note"), "a reminder is a System note")
        #expect(seen.contains("native-show-more"), "the report folds after a few lines")
        #expect(!seen.contains("native-notice-turn"), "none is a notice wall")
        #expect(!seen.contains("native-harness-note-body"), "the note is closed until asked for")

        #expect(NativePolishTests.click("section-header-harness-note", frames: frames, in: window), "the note has a toggle")
        await T.settle(window, 600)
        #expect(seen.contains("native-harness-note-body"), "and opens")
    }
}

/// Opt-in (FARCOOLER_CAPTURE_OUT): the same rows at a narrow and a wide
/// width, light and dark, the note closed and open.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct NativeHarnessCaptures {
    @Test func harnessRows() async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for (width, size) in [(CGFloat(420), "narrow"), (CGFloat(1_000), "wide")] {
            for (name, appearance) in RealWindowCaptures.variants.prefix(2) {
                for opened in [false, true] {
                    let model = NativeAgentTests.model(try NativeAgentTests.terminal())
                    model.store.apply(try await model.store.ledger.page(NativeAgentTests.page(try NativeHarnessTests.rows())))
                    let frames = NativePolishTests.Frames()
                    let window = NativePolishTests.window(
                        width: width, height: 1_100,
                        NativePolishTests.Framed(frames: frames, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
                    window.appearance = NSAppearance(named: appearance)
                    await NativeAgentTests.settle(window, 300)
                    if opened {
                        NativePolishTests.click("section-header-harness-note", frames: frames, in: window)
                        await NativeAgentTests.settle(window, 600)
                    }
                    let image = try #require(RealWindowCaptures.windowImage(window), "screencapture -l isn't allowed here")
                    try #require(image.representation(using: .png, properties: [:]))
                        .write(to: URL(fileURLWithPath: out).appendingPathComponent("mac-harness-\(size)-\(name)\(opened ? "-note-open" : "").png"))
                    window.close()
                }
            }
        }
    }
}
