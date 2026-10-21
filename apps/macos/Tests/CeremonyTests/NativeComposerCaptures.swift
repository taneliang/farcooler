import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the native view's composer against a
/// runner with `compose` (ov-400), holding two lines and an image chip, in
/// the four appearances. The production `NativeAgentView` over a pane model
/// whose rows come through its own ledger, in a real titled window off every
/// screen, taken with `screencapture -l` of that window. Sends no input: the
/// image is pasted from a pasteboard of the test's own.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct NativeComposerCaptures {
    /// A small picture of a terminal, as a screenshot pasted would be.
    static func screenshot() -> Data {
        let size = NSSize(width: 320, height: 200)
        let image = NSImage(size: size, flipped: false) { rect in
            NSColor(calibratedRed: 0.11, green: 0.12, blue: 0.14, alpha: 1).setFill()
            rect.fill()
            let lines = ["$ cargo test", "running 42 tests", "test parse::empty ... FAILED", "error: 1 failed"]
            for (i, line) in lines.enumerated() {
                let color: NSColor = line.contains("FAILED") || line.hasPrefix("error") ? .systemRed : .white
                line.draw(at: NSPoint(x: 14, y: 160 - CGFloat(i) * 32), withAttributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 16, weight: .regular), .foregroundColor: color,
                ])
            }
            return true
        }
        let rep = NSBitmapImageRep(data: image.tiffRepresentation!)!
        return rep.representation(using: .png, properties: [:])!
    }

    @Test func composerWithTwoLinesAndAnImage() async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for (name, appearance) in RealWindowCaptures.variants {
            let model = NativeAgentTests.model(try NativeAgentTests.terminal(), sink: NativeAgentTests.StandInSink())
            model.rich = true
            model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([
                NativeAgentTests.row(0, "turn:p1", ["Turn": [
                    "prompt": "Why does the parser test fail?", "origin": "Typed", "started_ms": 1, "ended_ms": 9_000,
                    "duration_ms": 9_000, "outcome": "Finished", "background_running": 0, "activity": NSNull(),
                ]]),
                NativeAgentTests.row(1, "prose:1", ["Prose": [
                    "text": "The empty input reaches `parse` before the length check, so it indexes past the end.",
                    "conclusion": true, "at_ms": 9_000,
                ]]),
            ])))
            let window = NativeAgentTests.window(NativeAgentView(model: model, isFocused: true, showTerminal: {}))
            window.appearance = NSAppearance(named: appearance)
            await NativeAgentTests.settle(window)
            let text = try #require(NativeComposerTests.textView(in: window.contentView))
            let board = NSPasteboard(name: NSPasteboard.Name("fc-composer-capture-\(UUID().uuidString)"))
            board.clearContents()
            board.setData(Self.screenshot(), forType: .png)
            text.pasteboard = board
            text.paste(nil)
            NativeComposerTests.type(text, "Fix the empty-input case in parse.rs")
            NativeComposerTests.press(text, shift: true)
            NativeComposerTests.type(text, "and add a test for it, like this one:")
            try await Capture.wait(expecting: "the pasted image and the two typed lines", until: { model.images.count == 1 && model.draft.contains("\n") })
            await NativeAgentTests.settle(window, 600)
            try await Capture.keep(
                to: URL(fileURLWithPath: out).appendingPathComponent("composer-\(name).png"),
                expecting: "the pasted image and the two typed lines", until: { model.images.count == 1 && model.draft.contains("\n") }
            ) { RealWindowCaptures.windowImage(window)?.representation(using: .png, properties: [:]) }
            board.releaseGlobally()
            window.close()
        }
    }

    /// claude's suggestion (ov-409) in the empty box's place: a short one and
    /// one too long for the line, then the same box after Tab, in the four
    /// appearances. Tab is a key event handed to the text view, not posted.
    @Test func composerWithClaudesSuggestion() async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let long = "Add a regression test for the empty input case in parse.rs, run the whole suite, and then summarize what changed in the lexer"
        for (shown, words) in [("short", "Run the tests again"), ("long", long)] {
            for (name, appearance) in RealWindowCaptures.variants {
                let model = NativeAgentTests.model(try NativeAgentTests.terminal(), sink: NativeAgentTests.StandInSink())
                model.rich = true
                model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([
                    NativeAgentTests.row(0, "turn:p1", ["Turn": [
                        "prompt": "Why does the parser test fail?", "origin": "Typed", "started_ms": 1, "ended_ms": 9_000,
                        "duration_ms": 9_000, "outcome": "Finished", "background_running": 0, "activity": "Idle",
                        "suggestion": words,
                    ]]),
                    NativeAgentTests.row(1, "prose:1", ["Prose": [
                        "text": "The empty input reaches `parse` before the length check, so it indexes past the end.",
                        "conclusion": true, "at_ms": 9_000,
                    ]]),
                ])))
                let window = NativeAgentTests.window(NativeAgentView(model: model, isFocused: true, showTerminal: {}))
                window.appearance = NSAppearance(named: appearance)
                try await Capture.wait(expecting: "claude's suggestion in the box", until: { model.suggestion == words })
                await NativeAgentTests.settle(window, 400)
                try await Capture.keep(
                    to: URL(fileURLWithPath: out).appendingPathComponent("suggestion-\(shown)-\(name).png"),
                    expecting: "claude's suggestion in the box", until: { model.suggestion == words }
                ) { RealWindowCaptures.windowImage(window)?.representation(using: .png, properties: [:]) }
                if shown == "short" {
                    let text = try #require(NativeComposerTests.textView(in: window.contentView))
                    NativeComposerSuggestionTests.tab(text)
                    try await Capture.wait(expecting: "the suggestion taken into the draft", until: { model.draft == words })
                    await NativeAgentTests.settle(window, 400)
                    try await Capture.keep(
                        to: URL(fileURLWithPath: out).appendingPathComponent("suggestion-taken-\(name).png"),
                        expecting: "the suggestion taken into the draft", until: { model.draft == words }
                    ) { RealWindowCaptures.windowImage(window)?.representation(using: .png, properties: [:]) }
                }
                window.close()
            }
        }
    }

    /// claude's `Try "…"` example (ov-409) as the placeholder of a fresh
    /// session's box, in the four appearances.
    @Test func composerWithTheTryHint() async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for (name, appearance) in RealWindowCaptures.variants {
            let model = NativeAgentTests.model(try NativeAgentTests.terminal(), sink: NativeAgentTests.StandInSink())
            model.rich = true
            model.store.apply(try await model.store.ledger.page(NativeAgentTests.page([
                NativeAgentTests.row(0, "hint:composer", ["Hint": ["text": "Try \"how does <filepath> work?\""]]),
            ])))
            let window = NativeAgentTests.window(NativeAgentView(model: model, isFocused: true, showTerminal: {}))
            window.appearance = NSAppearance(named: appearance)
            try await Capture.wait(expecting: "the Try hint in the box", until: { model.hint != nil })
            await NativeAgentTests.settle(window, 400)
            try await Capture.keep(
                to: URL(fileURLWithPath: out).appendingPathComponent("hint-\(name).png"),
                expecting: "the Try hint in the box", until: { model.hint != nil }
            ) { RealWindowCaptures.windowImage(window)?.representation(using: .png, properties: [:]) }
            window.close()
        }
    }
}
