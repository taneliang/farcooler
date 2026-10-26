import CoreGraphics

/// Which edges of a composer's scrolled text fade (ov-789): the rule apart
/// from the text view, so it can be held by a test at the exact geometry that
/// broke, and a check of the result that knows nothing of the rule.
///
/// An edge fades only while TEXT is out of view past it, and never over the
/// caret's line. The bottom used to fade "while there's more below", and the
/// text's own inset counted as more: scrolled to the caret at the end of a
/// draft taller than the field, the text view stops with the caret's line on
/// the edge and the inset still under it, so the fade sat on that line and
/// cut the tails of its p's and g's, on every line typed past the field's
/// height.
///
/// With the caret's line inside an edge's reach and text still past that
/// edge, the edge doesn't fade: the text beyond it is cut at the edge rather
/// than faded, a trade made so the line being typed is always whole.
public struct ComposerFade: Equatable, Sendable {
    public var above = false
    public var below = false

    public static let none = ComposerFade()
    /// How far in from an edge its fade reaches.
    public static let depth: CGFloat = 8

    public init(above: Bool = false, below: Bool = false) {
        self.above = above
        self.below = below
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
        return ComposerFade(above: above, below: below)
    }

    /// Whether these edges' fade lies over the caret's line, from `caretTop`
    /// to `caretBottom`, in a view scrolled to `offset` and `height` tall:
    /// geometry only, whatever decided the edges, so a rule that fades the
    /// line being typed says so here.
    public func covers(caretTop: CGFloat, caretBottom: CGFloat, offset: CGFloat, height: CGFloat) -> Bool {
        let top = offset, bottom = offset + height
        let underTop = above && caretTop < top + Self.depth && caretBottom > top
        let underBottom = below && caretBottom > bottom - Self.depth && caretTop < bottom
        return underTop || underBottom
    }
}
