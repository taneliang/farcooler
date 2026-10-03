import SwiftUI

/// Every column's header in a workspace (ov-92): the navigator's, the
/// orchestrator's, and the jump bar over a task or a worktree. One height,
/// one text size and one divider, so their bottom edges are one straight
/// line across the window. The board's header was 40 pt beside the
/// orchestrator's 30 before (owner, 2 Oct).
enum ColumnHeader {
    /// The bar's height, its divider not counted: four rhythms, room for a
    /// 24 pt control with 4 pt each side.
    static let height: CGFloat = 4 * ColumnGrid.rhythm
    /// The one text size in a header: its title, a jump bar's every segment.
    static let textSize: CGFloat = 12
    /// A header's divider: the system's hairline.
    static let divider: CGFloat = 1

    /// A header's text, at the one size.
    static func font(_ weight: Font.Weight = .regular) -> Font { .system(size: textSize, weight: weight) }

    /// What the bar and its divider take together.
    static var total: CGFloat { height + divider }
}

extension View {
    /// This view as a column header: `ColumnHeader.height` tall, on the
    /// canvas, over the one divider. Every header is drawn through this, so
    /// none can come to differ (`ColumnHeaderTests`).
    func columnHeader() -> some View {
        VStack(spacing: 0) {
            self
                .frame(maxWidth: .infinity, alignment: .leading)
                .frame(height: ColumnHeader.height)
                .clipped()
                .background(WorkspaceStyle.canvas)
            Divider()
        }
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

    /// A segment's icon (the worktree's ⎇): the text's own size and weight.
    static func icon(_ role: Role, isHere: Bool = false) -> Style { style(role, isHere: isHere) }

    /// Each separator, and a menu's ⌄: one chevron, one size, one weight.
    static let chevron = Style(size: ColumnHeader.textSize - 3, weight: .semibold, tone: .tertiary)
    static let separatorGlyph = "chevron.right"
    static let menuGlyph = "chevron.down"
    /// Between a segment and a separator, each side.
    static let spacing: CGFloat = 6
}
