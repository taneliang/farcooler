import AppKit
import Foundation
import Testing

@testable import Far_Cooler

/// Return while an input method is composing is not a send (ov-162).
///
/// The task field (`SubmittingTextView`) and the palette's field had the guard;
/// the agent composer did not, so confirming a Japanese candidate with Return
/// sent the half-composed message to the agent.
@MainActor
struct ComposerIMETests {
    private func returnKey() throws -> NSEvent {
        try #require(
            NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
                windowNumber: 0, context: nil, characters: "\r",
                charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36))
    }

    private func composing() -> (ComposerTextView, sent: () -> Int) {
        let view = ComposerTextView()
        var sent = 0
        view.onSubmit = { sent += 1 }
        view.setMarkedText(
            "にほ", selectedRange: NSRange(location: 2, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0))
        return (view, { sent })
    }

    @Test func returnDuringCompositionDoesNotSend() throws {
        let (view, sent) = composing()
        #expect(view.hasMarkedText())
        view.keyDown(with: try returnKey())
        #expect(sent() == 0, "Return sent half-composed text")
    }

    @Test func theInsertNewlineRouteDoesNotSendEither() {
        let (view, sent) = composing()
        view.insertNewline(nil)
        #expect(sent() == 0)
    }

    /// The guard is only for composition: a plain Return still sends.
    @Test func returnWithNothingMarkedStillSends() throws {
        let view = ComposerTextView()
        var sent = 0
        view.onSubmit = { sent += 1 }
        view.keyDown(with: try returnKey())
        #expect(sent == 1)
    }
}
