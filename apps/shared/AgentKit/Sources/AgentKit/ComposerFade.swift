import CoreGraphics

/// Which edges of a composer's scrolled text fade (ov-789), and whether the
/// caret's line is under one: the rule apart from the text view, so it can
/// be held by a test at the exact geometry that broke.
///
/// An edge fades only while TEXT is out of view past it, and never over the
/// caret's line. The bottom used to fade "while there's more below", and the
/// text's own inset counted as more: scrolled to the caret at the end of a
/// draft taller than the field, the text view stops with the caret's line on
/// the edge and the inset still under it, so the fade sat on that line and
/// cut the tails of its p's and g's, on every line typed past the field's
/// height.
public struct ComposerFade: Equatable, Sendable {
    public var above = false
    public var below = false
    /// The caret's line is inside an edge that fades: read off the result,
    /// whatever decided the edges.
    public var caretFaded = false

    public static let none = ComposerFade()
    /// How far in from an edge its fade reaches.
    public static let depth: CGFloat = 8

    public init(above: Bool = false, below: Bool = false, caretFaded: Bool = false) {
        self.above = above
        self.below = below
        self.caretFaded = caretFaded
    }

    /// The edges to fade, for text scrolled to `offset` in a view `height`
    /// tall, over `content` of text with `insetTop` and `insetBottom` around
    /// it, and the caret's line from `caretTop` to `caretBottom` when there's
    /// a caret.
    public static func edges(
        offset: CGFloat, height: CGFloat, content: CGFloat, insetTop: CGFloat, insetBottom: CGFloat,
        caretTop: CGFloat? = nil, caretBottom: CGFloat? = nil
    ) -> ComposerFade {
        guard height > depth * 2 else { return .none }
        let top = offset, bottom = offset + height
        var above = top > insetTop + 0.5
        var below = bottom < content - insetBottom - 0.5
        if let caretBottom, caretBottom > bottom - depth { below = false }
        if let caretTop, caretTop < top + depth { above = false }
        let faded = (above && (caretTop.map { $0 < top + depth } ?? false))
            || (below && (caretBottom.map { $0 > bottom - depth } ?? false))
        return ComposerFade(above: above, below: below, caretFaded: faded)
    }
}
