import UIKit

/// The agent composer's text view (`ComposerTextView`), which also answers
/// ⌘↩ when it's asked to: the iPad's column composer sends with it (ov-348
/// review R2-3). The field's own key command, because the text view took ⌘↩
/// as a new line before a button's shortcut saw it.
final class ComposerField: UITextView {
    var onCommandReturn: (() -> Void)?

    /// Asked once, when the field first has a window: whether the reader had
    /// the keyboard in the composer this one replaces. Asked then, and not
    /// when the field is made, since the reader may put the keyboard away in
    /// between (ov-357).
    var shouldTakeFocus: (() -> Bool)?

    /// Told when the field leaves its window or is dismantled, so the model
    /// can tell a hand-off to another composer from a pane that's hidden.
    var onTakeDown: (() -> Void)?

    /// Set as the field is leaving its window or being dismantled, so the
    /// resign that comes with it isn't read as the reader putting the
    /// keyboard away. See `ComposerTextView.Coordinator`.
    private(set) var isBeingTakenDown = false

    func takeDown() {
        isBeingTakenDown = true
        onTakeDown?()
    }

    override func willMove(toWindow newWindow: UIWindow?) {
        if newWindow == nil { takeDown() }
        super.willMove(toWindow: newWindow)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil else { return }
        isBeingTakenDown = false
        if let ask = shouldTakeFocus {
            shouldTakeFocus = nil
            if ask() { becomeFirstResponder() }
        }
    }

    override var keyCommands: [UIKeyCommand]? {
        guard onCommandReturn != nil else { return super.keyCommands }
        let send = UIKeyCommand(title: "Send", action: #selector(commandReturn), input: "\r", modifierFlags: .command)
        send.wantsPriorityOverSystemBehavior = true
        return (super.keyCommands ?? []) + [send]
    }

    @objc private func commandReturn() { onCommandReturn?() }
}
