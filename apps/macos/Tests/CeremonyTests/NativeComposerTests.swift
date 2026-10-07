import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The native composer against a runner with `compose` (ov-400): Return
/// sends and Shift-Return breaks the line, an image pasted waits as a chip
/// and goes with the message, a slash command is the runner's to judge, and
/// each refusal has its sentence. Drawn in a real offscreen window, keys
/// handed to the box's own text view; nothing is typed into the system, and
/// the paste reads a pasteboard of the test's own, never the clipboard.
@MainActor
@Suite(.serialized)
struct NativeComposerTests {
    typealias Sink = NativeAgentTests.StandInSink

    /// The pane's model, its view in a window, and the box's text view.
    struct Composer {
        let model: NativePaneModel
        let sink: Sink
        let window: NSWindow
        let seen: NativeAgentTests.Seen
        let text: ComposerTextView
    }

    static func composer(rich: Bool, answer: Result<Bool, RunnerCore.Failure> = .success(false)) async throws -> Composer {
        let sink = Sink()
        await sink.set(answer)
        let model = NativeAgentTests.model(try NativeAgentTests.terminal(), sink: sink)
        model.rich = rich
        let seen = NativeAgentTests.Seen()
        let window = NativeAgentTests.window(
            NativeAgentTests.Probe(seen: seen, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
        await NativeAgentTests.settle(window)
        let text = try #require(Self.textView(in: window.contentView))
        return Composer(model: model, sink: sink, window: window, seen: seen, text: text)
    }

    static func textView(in view: NSView?) -> ComposerTextView? {
        guard let view else { return nil }
        if let text = view as? ComposerTextView { return text }
        return view.subviews.lazy.compactMap { textView(in: $0) }.first
    }

    /// Return, with or without Shift, as the key event the box's window
    /// would hand it. An `NSEvent` object only: nothing is posted.
    static func press(_ text: ComposerTextView, shift: Bool) {
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: shift ? .shift : [], timestamp: 0,
            windowNumber: text.window?.windowNumber ?? 0, context: nil, characters: "\r", charactersIgnoringModifiers: "\r",
            isARepeat: false, keyCode: 36)!
        text.keyDown(with: event)
    }

    /// Type `words` at the caret, as the input system inserts them.
    static func type(_ text: ComposerTextView, _ words: String) {
        text.insertText(words, replacementRange: text.selectedRange())
    }

    /// Wait, polling, for `condition`: at most 30 s, so a loaded runner
    /// isn't a failure.
    static func until(_ what: String, _ condition: () async -> Bool) async {
        let deadline = ContinuousClock.now + .seconds(30)
        while ContinuousClock.now < deadline {
            if await condition() { return }
            try? await Task.sleep(for: .milliseconds(20))
        }
        Issue.record("timed out waiting for \(what)")
    }

    static func png() -> Data {
        let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: 4, pixelsHigh: 4, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
            isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        return rep.representation(using: .png, properties: [:])!
    }

    /// A pasteboard of the test's own holding a PNG, as a screenshot copied
    /// to the clipboard puts it.
    static func pasteboard(with png: Data) -> NSPasteboard {
        let board = NSPasteboard(name: NSPasteboard.Name("fc-composer-test-\(UUID().uuidString)"))
        board.clearContents()
        board.setData(png, forType: .png)
        return board
    }

    @Test("Shift-Return breaks the line and Return sends both lines")
    func shiftReturnBreaksTheLine() async throws {
        let c = try await Self.composer(rich: true)
        defer { c.window.close() }
        Self.type(c.text, "fix the build")
        Self.press(c.text, shift: true)
        Self.type(c.text, "then the docs")
        #expect(c.model.draft == "fix the build\nthen the docs")
        #expect(await c.sink.sent.isEmpty, "Shift-Return sent")
        Self.press(c.text, shift: false)
        await Self.until("the send") { await !c.sink.sent.isEmpty && c.model.draft.isEmpty }
        #expect(await c.sink.sent == ["fix the build\nthen the docs"])
        #expect(c.model.issue == nil)
    }

    @Test("Without compose the box stays one line")
    func withoutComposeOneLine() async throws {
        let c = try await Self.composer(rich: false)
        defer { c.window.close() }
        Self.type(c.text, "fix the build")
        Self.press(c.text, shift: true)
        Self.type(c.text, "then the docs")
        #expect(c.model.draft == "fix the build then the docs")
        // And no image: a paste of one goes to the text view as usual.
        c.text.pasteboard = Self.pasteboard(with: Self.png())
        c.text.paste(nil)
        #expect(c.model.images.isEmpty)
    }

    @Test("A pasted image waits as a chip and goes with the message, Sent")
    func aPastedImageIsSent() async throws {
        let c = try await Self.composer(rich: true)
        defer { c.window.close() }
        let png = Self.png()
        c.text.pasteboard = Self.pasteboard(with: png)
        c.text.paste(nil)
        #expect(c.model.images.count == 1)
        await NativeAgentTests.settle(c.window)
        #expect(c.seen.ids.contains("native-image-chip"))
        #expect(c.seen.ids.contains("native-attach"), "the paperclip")

        // A second, taken out again.
        c.text.paste(nil)
        try #require(c.model.images.count == 2)
        c.model.detach(c.model.images[1].id)

        Self.type(c.text, "what's wrong here")
        Self.press(c.text, shift: false)
        await Self.until("the send") { await !c.sink.sent.isEmpty && c.model.images.isEmpty }
        #expect(await c.sink.sent == ["what's wrong here"])
        let sent = await c.sink.images
        #expect(sent.count == 1 && sent[0].map(\.mime) == ["image/png"] && sent[0].first?.data == png)
        #expect(c.model.draft.isEmpty && c.model.issue == nil && c.model.queued.isEmpty, "Sent: no Queued echo")
        await NativeAgentTests.settle(c.window)
        #expect(!c.seen.ids.contains("native-image-chip"))
    }

    @Test("Queued with an image, the echo gives way to the transcript's row")
    func aQueuedImageEchoSettles() async throws {
        let c = try await Self.composer(rich: true, answer: .success(true))
        defer { c.window.close() }
        c.text.pasteboard = Self.pasteboard(with: Self.png())
        c.text.paste(nil)
        Self.type(c.text, "and this")
        await c.model.send()
        #expect(c.model.queued == ["[Image] and this"])
        c.model.store.apply(try await c.model.store.ledger.page(NativeAgentTests.page([
            NativeAgentTests.row(0, "queued:1", ["Queued": ["text": "[Image #3] and this", "state": "Waiting", "at_ms": 1]]),
        ])))
        c.model.settleQueued()
        #expect(c.model.queued.isEmpty)
    }

    @Test("A slash command goes to the runner where it has compose")
    func aSlashCommandIsTheRunners() async throws {
        let c = try await Self.composer(rich: true)
        defer { c.window.close() }
        c.model.draft = "/init focus on the tests"
        await c.model.send()
        #expect(await c.sink.sent == ["/init focus on the tests"])
    }

    @Test("Each refusal says what happened and what to do")
    func refusalsHaveTheirSentences() async throws {
        let refused = { (what: String) in RunnerCore.Failure.refused("no", word: "resource-conflict", what: what) }
        #expect(NativePaneModel.issue(for: refused("handoff")) == .panel)
        #expect(NativePaneModel.issue(for: refused("images_too_large")) == .said(NativePaneModel.imagesTooLarge))
        #expect(NativePaneModel.issue(for: refused("command")) == .said(NativePaneModel.commandRefused))
        #expect(NativePaneModel.issue(for: refused("unconfirmed")) == .said(NativePaneModel.unconfirmed))
        #expect(NativePaneModel.unconfirmed.contains("Check the pane before sending it again"))
        for what in ["images", "image", "unconfirmable", "unsupported"] {
            guard case .said(let words) = NativePaneModel.issue(for: refused(what)) else {
                Issue.record("\(what): no sentence")
                continue
            }
            #expect(words != "The message wasn’t sent.", "\(what) has no sentence of its own")
        }

        // The panel's Handoff row, with Show Terminal, and the draft kept.
        let c = try await Self.composer(rich: true, answer: .failure(refused("handoff")))
        defer { c.window.close() }
        c.model.draft = "/model"
        await c.model.send()
        await NativeAgentTests.settle(c.window)
        #expect(c.model.issue == .panel && c.model.draft == "/model")
        #expect(c.seen.ids.contains("native-handoff") && c.seen.ids.contains("native-handoff-show-terminal"))
    }
}
