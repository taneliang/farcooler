import SwiftUI

// One visual language (ov-216). Everything a view needs to draw an edge, a
// fill, a rule or a piece of glass lives in this file, and
// `scripts/visual-tokens-lint.py` fails CI when a Mac or AgentKit view draws
// one by hand instead. The design is
// `.claude/agent/reports/ov-216/design.md`; the sentence it comes down to is
// "the chrome is frosted, the work is paper, and the controls float."
//
// Owner decisions behind the values (3 Oct 2026): the whole window plane is
// frosted; the terminal theme's hue tints cards only, never the plane; cards
// have no stroke; the focused pane is marked neutrally; "needs you" is amber.

// MARK: - Corner radius

/// The three corner radii every surface is drawn with, plus the capsule.
///
/// Steps, not numbers: a fourth value is a design change, made here, never in a
/// view. Use `.control`, `.card` and `.floating` (below) rather than building a
/// `RoundedRectangle` yourself, so the corners are continuous like the window's.
public enum Radius {
    /// Something the hand acts on inside a list or bar: a row's selection or
    /// hover, a field, an inline button, a segmented tab, a code span.
    public static let small: CGFloat = 6
    /// A card: a pane, the task document, a note, a tool block, a message.
    public static let medium: CGFloat = 10
    /// A free-floating surface: the palette, quick create, a banner, a tip.
    public static let large: CGFloat = 16

    /// The radius of a shape nested `padding` points inside a container whose
    /// corner is `outer`: the outer radius minus the padding, never below
    /// `small`, so a shape near a corner stays concentric and a shape far from
    /// one doesn't go square. Use it where `.concentric` (a live
    /// `ConcentricRectangle`) can't resolve a container shape, or to check one.
    public static func concentric(outer: CGFloat, padding: CGFloat) -> CGFloat {
        max(outer - padding, small)
    }
}

extension Shape where Self == RoundedRectangle {
    /// `Radius.small`, continuous. For rows, fields and inline buttons.
    public static var control: RoundedRectangle {
        RoundedRectangle(cornerRadius: Radius.small, style: .continuous)
    }
    /// `Radius.medium`, continuous. For cards.
    public static var card: RoundedRectangle {
        RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
    }
    /// `Radius.large`, continuous. For surfaces that float above the content.
    public static var floating: RoundedRectangle {
        RoundedRectangle(cornerRadius: Radius.large, style: .continuous)
    }
}

extension Shape where Self == ConcentricRectangle {
    /// Concentric with the nearest `.containerShape`, never tighter than
    /// `Radius.small`. Use for a shape that sits within twice its padding of its
    /// container's corner (a composer in a card, the first row in a palette).
    public static var concentric: ConcentricRectangle {
        ConcentricRectangle(corners: .concentric(minimum: .fixed(Radius.small)), isUniform: false)
    }
}

// MARK: - Spacing

/// The 8 pt rhythm, named. Space is what replaces a rule between two groups.
public enum Spacing {
    /// A glyph and its label; stacked lines in one row.
    public static let tight: CGFloat = 4
    /// Between rows' groups, and between cards in a stack.
    public static let group: CGFloat = 8
    /// A card's edge to its text.
    public static let inset: CGFloat = 12
    /// Above a section heading. This is what replaces a separator line.
    public static let section: CGFloat = 16
}

// MARK: - Fills and selection

/// The only grays and accent washes a view may fill with.
public enum Fill {
    /// The opacity of `inset`: 0.05, and 0.10 under Increase Contrast.
    public static func insetOpacity(_ contrast: ColorSchemeContrast) -> Double {
        contrast == .increased ? 0.10 : 0.05
    }
    /// A group inside content: a tool block, a note, a code box. A fill is the
    /// edge; never add a stroke.
    public static func inset(_ contrast: ColorSchemeContrast = .standard) -> Color {
        Color.primary.opacity(insetOpacity(contrast))
    }
    /// The wash under a row the pointer is on.
    public static let hover = Color.primary.opacity(0.05)

    /// The opacity of `selection`: accent 0.13, gray 0.09 when the window isn't
    /// key, and accent 0.25 under Increase Contrast.
    public static func selectionOpacity(active: Bool, contrast: ColorSchemeContrast) -> Double {
        switch (active, contrast == .increased) {
        case (true, true): 0.25
        case (true, false): 0.13
        case (false, _): 0.09
        }
    }
    /// The one selection look, for a selected row anywhere (sidebar, navigator,
    /// task rows, tabs). Command palette and menu rows keep the system's solid
    /// accent.
    public static func selection(active: Bool, contrast: ColorSchemeContrast = .standard) -> Color {
        let opacity = selectionOpacity(active: active, contrast: contrast)
        return active ? Color.accentColor.opacity(opacity) : Color.primary.opacity(opacity)
    }
}

// MARK: - Attention

/// The only colors that mean something went wrong or needs someone.
public enum Tint {
    /// "Needs you": marks, counts and the fill of an attention card (at 0.10).
    /// Words and pills use it too; the accent belongs only to the control that
    /// acts on the state (the link or button). Never color alone: pair it with a
    /// glyph or words.
    public static func attention(_ scheme: ColorScheme) -> Color { GlancePalette.amber(scheme) }
    /// The opacity of `attentionFill`: 0.10, and 0.20 under Increase Contrast,
    /// where `Fill.inset` itself doubles to 0.10 and a 0.10 wash over it would
    /// be lost.
    public static func attentionFillOpacity(_ contrast: ColorSchemeContrast) -> Double {
        contrast == .increased ? 0.20 : 0.10
    }
    /// The wash behind an attention card: `attention` at `attentionFillOpacity`.
    /// A view draws it with `.attentionSurface(in:)`, which reads the contrast
    /// and adds the outline.
    public static func attentionFill(_ scheme: ColorScheme, contrast: ColorSchemeContrast = .standard) -> Color {
        attention(scheme).opacity(attentionFillOpacity(contrast))
    }
    /// A failed turn, the system red.
    public static let failure = GlancePalette.failed
}

// MARK: - Surfaces

/// The four levels a surface can be: the material hierarchy.
public enum Surface: Sendable, Equatable {
    /// The frosted plane behind everything: the sidebar, the navigator, the
    /// gutters around cards. Draws nothing, because the window's own material
    /// shows through. Never paint an opaque fill over it.
    case window
    /// Opaque paper for work: terminal cards, diffs, the task document, the
    /// transcript. Never translucent, so text reads over any wallpaper and at
    /// any Liquid Glass setting.
    case content
    /// A group inside content (a tool block, a note, a code box): `Fill.inset`.
    /// Never a stroke as its only edge, never a material.
    case inset
    /// Floating above content: system Liquid Glass, for the palette, quick
    /// create, tips, banners and the composer group. Never on a card, a row or a
    /// header; never glass inside glass except members of one
    /// `GlassEffectContainer`; never a hand-made shadow.
    case floating
}

extension Surface {
    /// Whether the level paints an opaque fill, so a test can pin that
    /// `.content` is never translucent and `.window` draws nothing.
    public var paintsOpaque: Bool { self == .content }
    /// Whether the level draws anything of its own.
    public var drawsAnything: Bool { self != .window }
}

/// One family of floating surfaces: every member is `.surface(.floating, in:)`,
/// and the group lets neighboring glass blend and morph as one piece instead of
/// sampling each other. The only way a view draws glass inside glass. Members
/// share a corner radius, concentric with the card they rest in.
public struct FloatingGroup<Content: View>: View {
    private let spacing: CGFloat
    private let content: Content

    public init(spacing: CGFloat = Spacing.group, @ViewBuilder content: () -> Content) {
        self.spacing = spacing
        self.content = content()
    }

    public var body: some View {
        GlassEffectContainer(spacing: spacing) { content }
    }
}

private struct SurfaceModifier<S: Shape>: ViewModifier {
    let level: Surface
    let shape: S
    let content: Color
    @Environment(\.colorSchemeContrast) private var contrast

    func body(content view: Content) -> some View {
        switch level {
        case .window:
            view
        case .content:
            view.background(content, in: shape).overlay(edge)
        case .inset:
            view.background(Fill.inset(contrast), in: shape).overlay(edge)
        case .floating:
            view.glassEffect(.regular, in: shape)
        }
    }

    /// The edge an opaque card gets only under Increase Contrast: 1 px of the
    /// system separator color. Otherwise the color change is the edge.
    @ViewBuilder private var edge: some View {
        if contrast == .increased { shape.stroke(.separator, lineWidth: 1) }
    }
}

/// An attention card's wash, and its edge under Increase Contrast.
private struct AttentionSurfaceModifier<S: Shape>: ViewModifier {
    let shape: S
    let on: Bool
    @Environment(\.colorScheme) private var scheme
    @Environment(\.colorSchemeContrast) private var contrast

    func body(content: Content) -> some View {
        content
            .background(on ? Tint.attentionFill(scheme, contrast: contrast) : Color.clear, in: shape)
            .overlay {
                // The one outline an attention card gets: with Increase
                // Contrast on, a wash alone is too faint to find the card by.
                if on && contrast == .increased { shape.stroke(Tint.attention(scheme), lineWidth: 1) }
            }
    }
}

extension View {
    /// Draw this as an attention card (an approval, a failure, a tool call
    /// waiting on you) while `on`: `Tint.attentionFill`, and under Increase
    /// Contrast a stronger fill with a 1 pt `Tint.attention` outline. No outline
    /// otherwise; the glyph and words carry the state.
    public func attentionSurface<S: Shape>(in shape: S, when on: Bool = true) -> some View {
        modifier(AttentionSurfaceModifier(shape: shape, on: on))
    }

    /// The only way a view gets a background shape, fill, edge or glass. `fill`
    /// is for `.content` only: the Mac passes the theme-tinted document color,
    /// everyone else takes the platform's text background.
    public func surface<S: Shape>(
        _ level: Surface, in shape: S, fill: Color = Surface.contentFill
    ) -> some View {
        modifier(SurfaceModifier(level: level, shape: shape, content: fill))
    }
}

extension Surface {
    /// The opaque paper color when a surface doesn't name its own.
    public static var contentFill: Color {
        #if os(macOS)
            Color(nsColor: .textBackgroundColor)
        #elseif os(watchOS)
            Color.black
        #else
            Color(uiColor: .systemBackground)
        #endif
    }
}

// MARK: - Separator

/// Where a line is allowed. Everything else is spacing or a surface change.
/// Menus use `Divider()` inside `Menu`, `.contextMenu` or `Commands`, marked
/// `// style-exempt: menu`.
public enum Separator: Sendable, Equatable {
    /// Inside a grid of content: a diff gutter or hunk rule, a table cell.
    case grid
    /// Between two sub-panes one card holds and the user can resize.
    case split
    /// Between a field and the list it filters, inside a floating surface.
    case listEdge
}

private struct SeparatorModifier: ViewModifier {
    let role: Separator
    let edge: Edge
    @Environment(\.displayScale) private var scale

    func body(content: Content) -> some View {
        content.overlay(alignment: alignment) {
            Rectangle().fill(.separator)
                .frame(
                    width: edge.isVertical ? 1 / scale : nil,
                    height: edge.isVertical ? nil : 1 / scale)
                .padding(.horizontal, role == .listEdge && !edge.isVertical ? Radius.large : 0)
        }
    }

    private var alignment: Alignment {
        switch edge {
        case .top: .top
        case .bottom: .bottom
        case .leading: .leading
        case .trailing: .trailing
        }
    }
}

extension Edge {
    fileprivate var isVertical: Bool { self == .leading || self == .trailing }
}

extension View {
    /// A one-pixel rule in the system separator color, for a grid, a
    /// card-internal split or a floating list's edge. Not for a header, a
    /// footer, a card or a section: use `Spacing`.
    public func separator(_ role: Separator, edge: Edge = .bottom) -> some View {
        modifier(SeparatorModifier(role: role, edge: edge))
    }
}
