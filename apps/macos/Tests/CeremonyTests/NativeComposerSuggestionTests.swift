import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// claude's suggested next prompt in the native composer (ov-409): it shows
/// where "Message Claude" does, Tab takes it into the draft, and nothing is
/// sent until the person's own Return. Drawn in a real offscreen window with
/// keys handed to the box's own text view; the rows are the wire's JSON
/// through the real decoder.
@MainActor
@Suite(.serialized)
struct NativeComposerSuggestionTests {
    typealias Composer = NativeComposerTests.Composer

    /// A turn row as the runner writes it (`rows_fixture_tests.rs`), resting
    /// or `Busy`, with `suggestion` if any.
    static func turn(_ suggestion: String?, activity: String = "Idle") -> [String: Any] {
        var turn: [String: Any] = [
            "prompt": "go", "origin": "Typed", "started_ms": 1, "ended_ms": NSNull(), "duration_ms": NSNull(),
            "outcome": NSNull(), "background_running": 0, "activity": activity,
        ]
        if let suggestion { turn["suggestion"] = suggestion }
        return NativeAgentTests.row(0, "turn:p1", ["Turn": turn])
    }

    static func composer(_ suggestion: String?, activity: String = "Idle") async throws -> Composer {
        let c = try await NativeComposerTests.composer(rich: true)
        c.model.store.apply(try await c.model.store.ledger.page(NativeAgentTests.page([turn(suggestion, activity: activity)])))
        await NativeAgentTests.settle(c.window)
        return c
    }

    static func tab(_ text: ComposerTextView, modifiers: NSEvent.ModifierFlags = []) {
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: 0,
            windowNumber: text.window?.windowNumber ?? 0, context: nil, characters: "\t", charactersIgnoringModifiers: "\t",
            isARepeat: false, keyCode: 48)!
        text.keyDown(with: event)
    }

    @Test("The suggestion stands in the empty box's place and Tab takes it as a draft, unsent")
    func tabTakesTheSuggestion() async throws {
        let c = try await Self.composer("run the tests again")
        defer { c.window.close() }
        #expect(c.model.suggestion == "run the tests again")
        #expect(c.seen.ids.contains("native-suggestion"))
        Self.tab(c.text)
        #expect(c.model.draft == "run the tests again")
        #expect(c.text.string == "run the tests again")
        #expect(c.text.selectedRange().location == (c.text.string as NSString).length, "the caret ends after it")
        await NativeAgentTests.settle(c.window)
        #expect(!c.seen.ids.contains("native-suggestion"), "a draft replaces the placeholder")
        try await Task.sleep(for: .milliseconds(300))
        #expect(await c.sink.sent.isEmpty, "Tab never sends")
        // It is a draft: edited, then sent by the person's own Return.
        NativeComposerTests.type(c.text, " please")
        NativeComposerTests.press(c.text, shift: false)
        await NativeComposerTests.until("the send") { await !c.sink.sent.isEmpty }
        #expect(await c.sink.sent == ["run the tests again please"])
    }

    @Test("With no suggestion on offer, or typing, or a turn under way, there is none to take")
    func noSuggestionNoTake() async throws {
        let none = try await Self.composer(nil)
        defer { none.window.close() }
        #expect(none.model.suggestion == nil && !none.seen.ids.contains("native-suggestion"))
        #expect(none.text.onTab?() != true, "Tab is a Tab")

        let busy = try await Self.composer("run the tests again", activity: "Busy")
        defer { busy.window.close() }
        #expect(busy.model.suggestion == nil && !busy.seen.ids.contains("native-suggestion"), "mid-turn, claude's dim line is a hint")

        let typing = try await Self.composer("run the tests again")
        defer { typing.window.close() }
        NativeComposerTests.type(typing.text, "fix")
        #expect(typing.model.suggestion == nil)
        #expect(typing.text.onTab?() != true)
        #expect(typing.model.draft == "fix")
    }

    @Test("A modified Tab is not the suggestion's")
    func aModifiedTabIsLeftAlone() async throws {
        let c = try await Self.composer("run the tests again")
        defer { c.window.close() }
        Self.tab(c.text, modifiers: .shift)
        #expect(c.model.draft != "run the tests again")
    }

    @Test("claude's Try example is the placeholder as a hint: Tab takes nothing, and its row is not drawn")
    func theTryExampleIsAHint() async throws {
        let c = try await NativeComposerTests.composer(rich: true)
        defer { c.window.close() }
        let example = "Try \"how does <filepath> work?\""
        c.model.store.apply(try await c.model.store.ledger.page(NativeAgentTests.page([
            NativeAgentTests.row(0, "hint:composer", ["Hint": ["text": example]]),
        ])))
        await NativeAgentTests.settle(c.window)
        #expect(c.model.hint == example)
        #expect(c.model.suggestion == nil, "an example is not a prediction")
        #expect(!c.seen.ids.contains("native-suggestion"))
        #expect(!c.seen.ids.contains("native-row-hint:composer"), "the hint is not a transcript row")
        Self.tab(c.text)
        #expect(c.model.draft != example, "Tab took the example")
        #expect(await c.sink.sent.isEmpty)
        // And it empties with the box.
        c.model.draft = ""
        c.model.store.apply(try await c.model.store.ledger.page(NativeAgentTests.page([
            NativeAgentTests.row(0, "hint:composer", ["Hint": ["text": ""]]),
        ])))
        #expect(c.model.hint == nil)
    }
}
