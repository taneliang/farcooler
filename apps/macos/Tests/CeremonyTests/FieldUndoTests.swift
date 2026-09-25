import AppKit
import Testing

@testable import Far_Cooler

/// ⌘Z in the app's hand-built text fields.
///
/// The task composer, the agent composer and the palette are `NSTextView`s
/// made in code, which start with `allowsUndo` false, so none of them
/// registered any undo. These drive a text view configured the way those three
/// are, through `FieldUndo`, and ask it to undo.
@MainActor
struct FieldUndoTests {
    /// What each field's coordinator is, as far as undo goes.
    final class Delegate: NSObject, NSTextViewDelegate {
        let undo = FieldUndo()
        func undoManager(for view: NSTextView) -> UndoManager? { undo.manager }
    }

    /// A field as the three are built, with typing grouped by hand: a test
    /// has no event loop to close a group for it.
    private func field() -> (NSTextView, Delegate) {
        let delegate = Delegate()
        let view = NSTextView()
        view.delegate = delegate
        view.isRichText = false
        FieldUndo.enable(view)
        delegate.undo.manager.groupsByEvent = false
        return (view, delegate)
    }

    private func type(_ text: String, into view: NSTextView, _ delegate: Delegate) {
        delegate.undo.manager.beginUndoGrouping()
        view.insertText(text, replacementRange: view.selectedRange())
        delegate.undo.manager.endUndoGrouping()
    }

    @Test("Typing in a field can be undone")
    func typingCanBeUndone() {
        let (view, delegate) = field()
        type("hello", into: view, delegate)
        #expect(view.string == "hello")
        #expect(delegate.undo.manager.canUndo, "typing registered no undo")
        delegate.undo.manager.undo()
        #expect(view.string == "")
    }

    /// A submit clears the composer from outside the text system. The actions
    /// recorded before it describe text that is gone, so they are forgotten,
    /// rather than left for ⌘Z to apply to ranges that no longer exist.
    @Test("Text written from outside forgets the undo it replaced")
    func textWrittenFromOutsideForgetsTheUndoItReplaced() {
        let (view, delegate) = field()
        type("a long prompt", into: view, delegate)
        #expect(delegate.undo.manager.canUndo)
        delegate.undo.replace(view, with: "")
        #expect(view.string == "")
        #expect(!delegate.undo.manager.canUndo, "the cleared field can still undo into old text")
    }
}
