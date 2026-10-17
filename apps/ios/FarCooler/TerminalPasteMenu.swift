import UIKit

/// Paste on a terminal pane (ov-604): the system edit menu and Command-V.
///
/// `KeystrokeSink` is the first responder and forwards `canPerformAction` and
/// `paste(_:)` here, so a hardware keyboard's Command-V and the menu that a
/// press on empty terminal opens answer to one rule: Paste exists only where
/// typing is allowed and the clipboard holds text. A read-only grant never
/// sees it. `hasStrings` is asked rather than the string itself, so offering
/// the menu never trips the system's paste prompt.
@MainActor
final class TerminalPasteMenu {
    /// Whether this phone's grant lets it type (`terminal.write` needs Control).
    var mayType: () -> Bool = { false }
    var onPaste: (String) -> Void = { _ in }

    func install(on view: UIView) {
        view.addInteraction(UIEditMenuInteraction(delegate: nil))
    }

    var isOffered: Bool { mayType() && UIPasteboard.general.hasStrings }

    /// Open the menu at a press. The view takes first responder first, since
    /// the menu asks it which actions apply, as a text view does.
    func present(in view: UIView, at point: CGPoint) {
        guard isOffered, view.becomeFirstResponder() else { return }
        let menu = view.interactions.compactMap { $0 as? UIEditMenuInteraction }.first
        menu?.presentEditMenu(with: UIEditMenuConfiguration(identifier: nil, sourcePoint: point))
    }

    func paste() {
        guard mayType(), let text = UIPasteboard.general.string, !text.isEmpty else { return }
        onPaste(text)
    }
}
