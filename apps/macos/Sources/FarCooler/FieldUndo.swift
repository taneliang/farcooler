import AppKit

/// Undo for the app's hand-built text fields: the task composer, the agent
/// composer, and the palette's query.
///
/// An `NSTextView` made in code has `allowsUndo` false, so those three
/// registered no undo at all. ⌘Z did nothing in the one place people type the
/// most, and Edit ▸ Redo was never enabled there. That is also why ⇧⌘Z, which
/// was then Zoom Pane as well as Redo, fell through to zooming the pane behind
/// the field.
///
/// Each field gets an undo manager of its OWN rather than the window's, handed
/// over by its delegate (`NSTextViewDelegate.undoManager(for:)`). Two reasons:
///
/// - These fields are also written from outside, when the SwiftUI binding
///   changes: a submit clears the composer, a draft is restored. That write
///   goes around the text system's undo, so every action recorded before it
///   describes ranges in text that is no longer there, and undoing one raises
///   inside AppKit. `replace(_:with:)` forgets them first.
/// - Forgetting them must not wipe the undo history of every other field in
///   the window, which is what clearing the window's manager would do.
final class FieldUndo {
    let manager = UndoManager()

    /// Turn undo on for a field whose delegate answers `undoManager(for:)`
    /// with `manager`.
    static func enable(_ view: NSTextView) {
        view.allowsUndo = true
    }

    /// Write text into the field from outside the text system, forgetting
    /// the undo actions that described the text it replaces.
    func replace(_ view: NSTextView, with text: String) {
        manager.removeAllActions()
        view.string = text
    }
}
