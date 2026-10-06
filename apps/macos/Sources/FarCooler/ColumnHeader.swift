import SwiftUI

/// Every column's header in a workspace (ov-92): the navigator's, the
/// orchestrator's, and the jump bar over a task or a worktree. One height
/// and one text size, so their bottom edges are one straight line across
/// the window. The board's header was 40 pt beside the orchestrator's 30
/// before (owner, 2 Oct). No rule under it: a header is not one of the
/// places a line is allowed (`Separator`), and the change of surface is its
/// edge (ov-216 design, section 9; review 1004i M2).
enum ColumnHeader {
    /// The bar's height: four rhythms, room for a 24 pt control with 4 pt
    /// each side.
    static let height: CGFloat = 4 * ColumnGrid.rhythm
    /// The one text size in a header: its title, a jump bar's every segment.
    static let textSize: CGFloat = 12
    /// A header's text, at the one size.
    static func font(_ weight: Font.Weight = .regular) -> Font { .system(size: textSize, weight: weight) }

    /// What the header takes: its bar, with no rule under it.
    static var total: CGFloat { height }
}

extension View {
    /// This view as a column header: `ColumnHeader.height` tall, with no
    /// rule. Every header is drawn through this, so none can come to differ
    /// (`ColumnHeaderTests`).
    func columnHeader() -> some View {
        self
            .frame(maxWidth: .infinity, alignment: .leading)
            .frame(height: ColumnHeader.height)
            .clipped()
    }
}

/// The jump bar's type (ov-92, owner, 2 Oct), as Xcode's: every segment at
/// the one header size on one baseline; the ancestors secondary and
/// regular, the current one primary; the separators one chevron, tertiary;
/// a segment's icon at the text's size; a menu's ⌄ the separators' weight.
/// As values, so `ColumnHeaderTests` can say each segment is drawn in it.
enum JumpBar {
    enum Role: CaseIterable { case ancestor, current, menu }
    enum Tone: Equatable { case primary, secondary, tertiary }

    struct Style: Equatable {
        var size: CGFloat
        var weight: Font.Weight
        var tone: Tone

        var font: Font { .system(size: size, weight: weight) }
        var color: HierarchicalShapeStyle {
            switch tone {
            case .primary: .primary
            case .secondary: .secondary
            case .tertiary: .tertiary
            }
        }
    }

    /// A segment's text: an ancestor's, the current one's, or a menu's,
    /// which is current only where it stands for where you are.
    static func style(_ role: Role, isHere: Bool = false) -> Style {
        switch role {
        case .ancestor: Style(size: ColumnHeader.textSize, weight: .regular, tone: .secondary)
        case .current: Style(size: ColumnHeader.textSize, weight: .medium, tone: .primary)
        case .menu:
            isHere ? style(.current) : Style(size: ColumnHeader.textSize, weight: .regular, tone: .secondary)
        }
    }

    /// The room a segment's or a menu row's icon has, so every symbol, wide
    /// or narrow, leaves its title at one edge (ov-328): a hair over the
    /// widest of the glyphs `OneTreeGlyph` names at the text's size.
    static let glyphWidth: CGFloat = 14
    /// Between a segment's icon column and its title (ov-351): 5 pt. An
    /// `NSPathControl` at 13 pt puts its icon's edge 5.3 pt from the title's
    /// origin (measured: 6.5 pt ink to ink); 3 pt read as touching.
    static let glyphGap: CGFloat = 5

    /// A segment's icon (the worktree's ⎇): the text's own size and weight.
    static func icon(_ role: Role, isHere: Bool = false) -> Style { style(role, isHere: isHere) }

    /// Each separator, and a menu's ⌄: one chevron, one size, one weight.
    static let chevron = Style(size: ColumnHeader.textSize - 3, weight: .semibold, tone: .tertiary)
    static let separatorGlyph = "chevron.right"
    static let menuGlyph = "chevron.down"
    /// Between one piece and the next: none. Each piece is a cell that carries
    /// its own room (ov-290), so a caret's gap to a separator is the two cells'
    /// padding, the same wherever it falls.
    static let spacing: CGFloat = 0
    /// The height of every piece's cell, labels, carets and separators alike:
    /// a control's square (`SidebarGrid.control`), so every piece centers on
    /// one line and a hover backing is as tall as the hit area it stands for.
    static let cell: CGFloat = SidebarGrid.control
    /// A separator's cell: the room its glyph has either side, a step and a half
    /// of the vertical rhythm.
    static let separatorWidth: CGFloat = ColumnGrid.rhythm * 1.5
    /// A plain (non-control) label's inset, as a label button's: so a segment
    /// without a link starts where one with it does.
    static let labelInset: CGFloat = 3
}
