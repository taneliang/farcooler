import CoreGraphics
import Testing

@testable import AgentKit

// ov-789: the composer's edge fade, at the geometry the owner saw cut the
// tails of the caret's line. The text view itself is held by the iOS UI
// tests (`GlassUITests`), whose simulator doesn't always reach this geometry:
// it depends on whether the field is at its six lines or a share of the room
// above the keyboard.

@Suite struct ComposerFadeTests {
    /// Body text on an iPhone: a 22 pt line, the field's 2 pt inset, six
    /// lines tall, eight lines of text.
    private let line: CGFloat = 22
    private let inset: CGFloat = 2
    private var height: CGFloat { line * 6 + 2 * inset }
    private var content: CGFloat { line * 8 + 2 * inset }

    /// **Typing at the end of a long draft never fades the caret's line.**
    /// UIKit scrolls the caret into view and stops with the bottom inset still
    /// out of view under it: the old rule called that "more below".
    @Test func theCaretsLineAtTheEndIsNeverFaded() {
        let offset = content - height - inset
        let caretTop = content - inset - line
        let fade = ComposerFade.edges(
            offset: offset, height: height, content: content, insetTop: inset, insetBottom: inset,
            caretTop: caretTop, caretBottom: caretTop + line)
        #expect(!fade.below, "the bottom fades over the caret's line, with only the inset below it")
        #expect(fade.above, "the lines out of view above don't fade")
        #expect(!fade.caretFaded)
    }

    /// The inset alone below, with no caret (the field not being typed in):
    /// nothing below to fade toward.
    @Test func theInsetAloneIsNotMoreBelow() {
        let fade = ComposerFade.edges(
            offset: content - height - inset, height: height, content: content, insetTop: inset, insetBottom: inset)
        #expect(!fade.below)
    }

    /// **Text out of view below fades the bottom**, and the top doesn't fade
    /// at the top of the text.
    @Test func textBelowFadesTheBottom() {
        let fade = ComposerFade.edges(
            offset: 0, height: height, content: content, insetTop: inset, insetBottom: inset,
            caretTop: inset, caretBottom: inset + line)
        #expect(fade.below, "two lines out of view below and no fade")
        #expect(!fade.above)
        #expect(!fade.caretFaded)
    }

    /// The caret on the bottom line with text still below it: that edge
    /// stands down rather than fade the line being typed.
    @Test func theCaretsLineKeepsTextBelowFromFadingIt() {
        let caretTop = height - inset - line
        let fade = ComposerFade.edges(
            offset: 0, height: height, content: content, insetTop: inset, insetBottom: inset,
            caretTop: caretTop, caretBottom: caretTop + line)
        #expect(!fade.below)
        #expect(!fade.caretFaded)
    }

    /// Everything in view: no fade at either edge.
    @Test func allInViewFadesNothing() {
        let fits = line * 3 + 2 * inset
        let fade = ComposerFade.edges(offset: 0, height: fits, content: fits, insetTop: inset, insetBottom: inset)
        #expect(fade == .none)
    }
}
