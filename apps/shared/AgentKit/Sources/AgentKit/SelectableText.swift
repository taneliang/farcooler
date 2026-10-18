#if os(iOS)
import SwiftUI
import UIKit

// Range selection in a conversation's text on the iPhone (ov-609).
//
// `.textSelection(.enabled)` on a SwiftUI `Text` offers, on iOS, a long press
// that copies the WHOLE text: no handles, no word, no range. Messages and
// Notes select a word on a long press and grow the selection with handles,
// and only a text view does that. So a conversation's words are drawn by a
// non-editable `UITextView`, sized to its text the way `Text` is, with its
// fonts resolved from the SwiftUI environment and its links opened through
// SwiftUI's `openURL`, as `Text`'s are.
//
// Each piece (a paragraph run, a list item, a message, a tool's output) is its
// own view, so a selection stays within one piece, as it does in a Messages
// bubble.

extension EnvironmentValues {
    /// Whether text under here selects ranges (`SelectableText`): set by the
    /// iPhone's conversation, where a long press on a word selects it.
    @Entry public var selectsRanges = false
    /// The color `SelectableText` draws a conversation's words in, where a
    /// row sets one (white on the person's bubble, secondary for machinery);
    /// nil for the label color. A `UIViewRepresentable` can't read the
    /// foreground style SwiftUI's `Text` would inherit, so it's said here.
    @Entry public var selectableColor: UIColor? = nil
}

/// Text that selects ranges, with handles and the system's edit menu.
public struct SelectableText: UIViewRepresentable {
    public let text: AttributedString
    /// The text's color: the label colors by default.
    public var color: UIColor
    /// Links' color, and whether they're underlined (on a colored bubble).
    public var linkColor: UIColor?
    public var underlinesLinks = false
    public var lineSpacing: CGFloat = 0
    /// Lines shown before the rest is cut with an ellipsis; nil for all.
    public var lineLimit: Int?
    /// The selection's and the handles' color.
    public var tint: UIColor?
    /// What VoiceOver says, where it isn't the text itself.
    public var spoken: String?

    public init(
        _ text: AttributedString, color: UIColor = .label, linkColor: UIColor? = nil, underlinesLinks: Bool = false,
        lineSpacing: CGFloat = 0, lineLimit: Int? = nil, tint: UIColor? = nil, spoken: String? = nil
    ) {
        self.text = text
        self.color = color
        self.linkColor = linkColor
        self.underlinesLinks = underlinesLinks
        self.lineSpacing = lineSpacing
        self.lineLimit = lineLimit
        self.tint = tint
        self.spoken = spoken
    }

    public init(verbatim string: String, color: UIColor = .label, lineLimit: Int? = nil, tint: UIColor? = nil) {
        self.init(AttributedString(string), color: color, lineLimit: lineLimit, tint: tint)
    }

    public func makeCoordinator() -> Coordinator { Coordinator() }

    public func makeUIView(context: Context) -> SelectableTextView {
        let view = SelectableTextView()
        view.isEditable = false
        view.isSelectable = true
        view.isScrollEnabled = false
        view.backgroundColor = .clear
        view.textContainerInset = .zero
        view.textContainer.lineFragmentPadding = 0
        view.dataDetectorTypes = []
        view.delegate = context.coordinator
        view.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        view.setContentHuggingPriority(.defaultHigh, for: .vertical)
        return view
    }

    public func updateUIView(_ view: SelectableTextView, context: Context) {
        context.coordinator.openURL = context.environment.openURL
        let attributed = Self.attributed(
            text, environment: context.environment, color: color, lineSpacing: lineSpacing)
        if view.attributedText != attributed { view.attributedText = attributed }
        view.textContainer.maximumNumberOfLines = lineLimit ?? 0
        view.textContainer.lineBreakMode = lineLimit == nil ? .byWordWrapping : .byTruncatingTail
        var links: [NSAttributedString.Key: Any] = [:]
        if let linkColor { links[.foregroundColor] = linkColor }
        if underlinesLinks { links[.underlineStyle] = NSUnderlineStyle.single.rawValue }
        view.linkTextAttributes = links.isEmpty ? nil : links
        view.tintColor = tint
        view.spoken = spoken ?? String(text.characters)
    }

    public func sizeThatFits(_ proposal: ProposedViewSize, uiView view: SelectableTextView, context: Context) -> CGSize? {
        let width = proposal.width.flatMap { $0.isFinite ? $0 : nil } ?? 10_000
        let fitted = view.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude))
        // As wide as its longest line, as `Text` is, so a bubble hugs its words.
        let used = view.attributedText.boundingRect(
            with: CGSize(width: width, height: .greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading], context: nil
        ).width
        return CGSize(width: min(ceil(used), width), height: ceil(fitted.height))
    }

    /// `text` with its fonts and colors as UIKit draws them: the
    /// environment's font for any run without its own, bold, italic and code
    /// from Markdown's inline intents, and `color` for any run without its own.
    static func attributed(
        _ text: AttributedString, environment: EnvironmentValues, color: UIColor, lineSpacing: CGFloat
    ) -> NSAttributedString {
        let context = environment.fontResolutionContext
        let base = UIFont(descriptor: ((environment.font ?? .body).resolve(in: context).ctFont as UIFont).fontDescriptor, size: 0)
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = lineSpacing
        let out = NSMutableAttributedString()
        for run in text.runs {
            var font = run.swiftUI.font.map { UIFont(descriptor: ($0.resolve(in: context).ctFont as UIFont).fontDescriptor, size: 0) } ?? base
            var attributes: [NSAttributedString.Key: Any] = [.paragraphStyle: paragraph]
            if let intent = run.inlinePresentationIntent {
                var traits = font.fontDescriptor.symbolicTraits
                if intent.contains(.stronglyEmphasized) { traits.insert(.traitBold) }
                if intent.contains(.emphasized) { traits.insert(.traitItalic) }
                if let styled = font.fontDescriptor.withSymbolicTraits(traits) { font = UIFont(descriptor: styled, size: 0) }
                if intent.contains(.code) {
                    font = .monospacedSystemFont(ofSize: font.pointSize * 0.94, weight: traits.contains(.traitBold) ? .semibold : .regular)
                }
                if intent.contains(.strikethrough) { attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
            }
            attributes[.font] = font
            attributes[.foregroundColor] = run.swiftUI.foregroundColor.map { UIColor($0) } ?? color
            if let link = run.link { attributes[.link] = link }
            out.append(NSAttributedString(string: String(text[run.range].characters), attributes: attributes))
        }
        return out
    }

    public final class Coordinator: NSObject, UITextViewDelegate {
        var openURL: OpenURLAction?

        /// A tap on a link opens it as SwiftUI's `Text` would, through the
        /// environment's `openURL`: a task key opens its task in the app.
        public func textView(
            _ textView: UITextView, primaryActionFor textItem: UITextItem, defaultAction: UIAction
        ) -> UIAction? {
            guard case let .link(url) = textItem.content, let openURL else { return defaultAction }
            return UIAction { _ in openURL(url) }
        }

        /// A long press on a link: Open Link and Copy Link, with no preview, and
        /// Open Link goes through `openURL` as a tap does, so a task key opens its
        /// task in the app rather than leaving it (review 1).
        public func textView(
            _ textView: UITextView, menuConfigurationFor textItem: UITextItem, defaultMenu: UIMenu
        ) -> UITextItem.MenuConfiguration? {
            guard case let .link(url) = textItem.content, let openURL else {
                return UITextItem.MenuConfiguration(menu: defaultMenu)
            }
            return UITextItem.MenuConfiguration(menu: UIMenu(children: [
                UIAction(title: "Open Link", image: UIImage(systemName: "safari")) { _ in openURL(url) },
                UIAction(title: "Copy Link", image: UIImage(systemName: "link")) { _ in UIPasteboard.general.url = url },
            ]))
        }
    }
}

/// The text view under `SelectableText`. VoiceOver reads it as the static
/// text it is, by its words, and its selection and Copy are the system's.
public final class SelectableTextView: UITextView {
    var spoken = ""

    override public var accessibilityLabel: String? {
        get { spoken }
        set {}
    }

    override public var accessibilityValue: String? {
        get { nil }
        set {}
    }

    override public var accessibilityTraits: UIAccessibilityTraits {
        get { .staticText }
        set {}
    }

    // Copy, Select All, Share and Look Up, as a read-only text offers;
    // nothing that would change it.
    override public func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        if action == #selector(cut(_:)) || action == #selector(paste(_:)) || action == #selector(delete(_:)) { return false }
        return super.canPerformAction(action, withSender: sender)
    }
}
#endif
