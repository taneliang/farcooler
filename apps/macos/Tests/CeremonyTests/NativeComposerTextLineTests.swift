import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The message box's text sits on the box's vertical center (ov-470). An
/// earlier check measured the box's frame, which was right; the text line
/// inside it was 5 pt low, so this one measures the line itself.
@MainActor
@Suite(.serialized)
struct NativeComposerTextLineTests {
    typealias T = NativeAgentTests
    typealias P = NativePolishTests

    /// The first line's rect, in the window content's top-left coordinates,
    /// as the layout manager has it for the first glyph.
    static func firstLine(of text: ComposerTextView, in content: NSView) -> CGRect {
        let manager = text.layoutManager!
        manager.ensureLayout(for: text.textContainer!)
        // An empty box has no glyph: its one line is the extra fragment.
        var rect = text.string.isEmpty ? manager.extraLineFragmentUsedRect : manager.lineFragmentUsedRect(forGlyphAt: 0, effectiveRange: nil)
        rect.origin.x += text.textContainerOrigin.x
        rect.origin.y += text.textContainerOrigin.y
        let inContent = text.convert(rect, to: content)
        return content.isFlipped ? inContent : CGRect(x: inContent.minX, y: content.bounds.height - inContent.maxY, width: inContent.width, height: inContent.height)
    }

    @Test("One line, empty or typed, is centered in the box with the paperclip, within 1 pt; two lines leave equal room above and below (ov-470)")
    func theTextLineSitsOnTheBoxCenter() async throws {
        for (stop, words) in [(false, ""), (false, "Fix it"), (true, "Fix it")] {
            let model = T.model(try T.terminal())
            model.rich = true
            model.draft = words
            let frames = P.Frames()
            let window = P.window(width: 720, height: 600, P.Framed(frames: frames, content: NativeAgentView(model: model, isFocused: true, showTerminal: {})))
            defer { window.close() }
            await T.settle(window, 300)
            let content = try #require(window.contentView)
            let text = try #require(NativeComposerTests.textView(in: content))
            let box = try #require(frames.views["native-composer-box"])
            let clip = try #require(frames.views["native-attach"])
            let line = Self.firstLine(of: text, in: content)
            let note = "draft '\(words)' stop \(stop): box \(box), paperclip \(clip), line \(line)"
            #expect(abs(line.midY - box.midY) <= 1, "the text line's center \(line.midY) is the box's \(box.midY): \(note)")
            #expect(abs(clip.midY - box.midY) <= 1, "the paperclip's center is the box's: \(note)")
            #expect(abs(line.midY - clip.midY) <= 1, "the text and the paperclip share a center: \(note)")

            // A second line: the text block, not its first line, is centered.
            NativeComposerTests.type(text, words.isEmpty ? "one" : "")
            NativeComposerTests.press(text, shift: true)
            NativeComposerTests.type(text, "two")
            await T.settle(window, 300)
            let first = Self.firstLine(of: text, in: content)
            let last = text.convert(text.layoutManager!.lineFragmentUsedRect(forGlyphAt: text.string.utf16.count - 1, effectiveRange: nil), to: content)
            let lastTop = content.isFlipped ? last.minY : content.bounds.height - last.maxY
            let lastBottom = lastTop + last.height
            let grown = try #require(frames.views["native-composer-box"])
            let above = first.minY - grown.minY
            let below = grown.maxY - lastBottom
            #expect(abs(above - below) <= 1, "two lines: \(above) above and \(below) below: box \(grown), first \(first)")
        }
    }

    @Test("Sending clears the box and it returns to one line's height, as a restored empty draft does (ov-470)")
    func theBoxShrinksWhenCleared() async throws {
        let c = try await NativeComposerTests.composer(rich: true)
        defer { c.window.close() }
        let one = ComposerField.lineHeight
        func height() -> CGFloat { c.text.enclosingScrollView?.frame.height ?? 0 }
        // Scroll bars that take room, as a Mac with no trackpad or mouse has
        // (CI's runner): text taller than the box brings one in while it is
        // measured, which narrowed the text mid-layout and measured three
        // restored lines as two.
        c.text.enclosingScrollView?.scrollerStyle = .legacy
        await NativeAgentTests.settle(c.window, 200)
        #expect(abs(height() - one) <= 1, "empty: \(height())")
        NativeComposerTests.type(c.text, "one")
        for word in ["two", "three", "four"] {
            NativeComposerTests.press(c.text, shift: true)
            NativeComposerTests.type(c.text, word)
        }
        await NativeAgentTests.settle(c.window, 300)
        #expect(height() > one * 3, "four lines are \(height()) tall")
        await c.model.send()
        await NativeAgentTests.settle(c.window, 400)
        #expect(c.model.draft.isEmpty && c.text.string.isEmpty, "sent: draft '\(c.model.draft)'")
        #expect(abs(height() - one) <= 1, "after send the box is \(height()), not one line's \(one)")
        // A restore from outside, to several lines and back to none.
        c.model.draft = "a\nb\nc"
        await NativeAgentTests.settle(c.window, 300)
        #expect(abs(height() - one * 3) <= 1, "restored three lines: \(height()), one line \(one), text \(c.text.string.debugDescription)")
        c.model.draft = ""
        await NativeAgentTests.settle(c.window, 300)
        #expect(abs(height() - one) <= 1, "restored empty: \(height())")
    }
}
