import UIKit

/// The agent composer's text view (`ComposerTextView`), which also answers
/// ⌘↩ when it's asked to: the iPad's column composer sends with it (ov-348
/// review R2-3). The field's own key command, because the text view took ⌘↩
/// as a new line before a button's shortcut saw it.
final class ComposerField: UITextView {
    var onCommandReturn: (() -> Void)?

    /// Whether the composer this one replaces had the keyboard: set when the
    /// field is made, acted on once it's in a window, where it can take it
    /// (ov-357).
    var wantsFocusOnArrival = false

    /// Set as the field is leaving its window or being dismantled, so the
    /// resign that comes with it isn't read as the reader putting the
    /// keyboard away. See `ComposerTextView.Coordinator`.
    var isBeingTakenDown = false

    override func willMove(toWindow newWindow: UIWindow?) {
        if newWindow == nil { isBeingTakenDown = true }
        super.willMove(toWindow: newWindow)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil else { return }
        isBeingTakenDown = false
        if wantsFocusOnArrival {
            wantsFocusOnArrival = false
            becomeFirstResponder()
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
