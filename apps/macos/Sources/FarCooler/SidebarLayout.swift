import AgentKit
import SwiftUI

/// The four columns the sidebar and the board column both lay out on, Finder
/// and Mail style (ov-83), measured from the column's leading edge: every
/// chevron, icon and text start sits on one of these, and nothing falls
/// between. One 18 pt step apart, the width of a disclosure chevron's cell,
/// so a row one level in starts exactly one column further over.
///
/// The owner's screenshot of 2 October counted six left edges down the
/// sidebar, among them a repository's name 12 pt in from everything else,
/// pushed there by a chevron cell that was drawn invisibly while open.
/// `GridGeometryTests` reads where each row type's marks really land, through
/// `gridMark(_:_:)`, and fails on any that isn't one of these.
enum ColumnGrid {
    /// One column to the next.
    static let step: CGFloat = 18
    /// The first column: the sidebar's title, its search, a repository's
    /// name, a workspace's chevron; the board's section chevrons.
    static let a: CGFloat = 16
    /// The second: a workspace's glyph, a worktree's chevron, Needs You's
    /// text; the board's section titles, its summary and its header title.
    static let b: CGFloat = a + step
    /// The third: a workspace's name, a worktree's branch glyph.
    static let c: CGFloat = b + step
    /// The fourth: a worktree's title and branch, a terminal's dot.
    static let d: CGFloat = c + step

    /// Column `n`, counting `a` as 0: the columns past `d` keep the step.
    static func column(_ n: Int) -> CGFloat { a + CGFloat(n) * step }

    /// Whether `x` is a column, to within a rounding error.
    static func isColumn(_ x: CGFloat) -> Bool {
        let n = ((x - a) / step).rounded()
        return n >= 0 && abs(x - column(Int(n))) < 0.5
    }

    /// The vertical base: row heights and the space between sections are
    /// multiples of it.
    static let rhythm: CGFloat = 8
    /// A one-line row: a workspace, Needs You, a board section's heading.
    static let rowHeight: CGFloat = 3 * rhythm
    /// A two-line row: a worktree, its title over its branch.
    static let twoLineRowHeight: CGFloat = 5 * rhythm
}

/// The navigator's grid: two lines down the board column, and a box that
/// breathes past them (ov-177 round 2, ov-230).
///
/// - `edge`: where a section's chevron starts, and the margin the column's
///   content is measured from.
/// - `text`: where every word starts. The header's title, a section's and a
///   group's title, a task's key, the filter's text and the orchestrator's
///   title all start here.
/// - `outset`: how far a box (the filter field, the orchestrator's card, a
///   row's selection, the focus ring) reaches past `edge` into the margin,
///   on both sides: half of it. The owner's screenshot of 3 October
///   ("hella cramped") showed boxes sitting exactly on the grid, their glyphs
///   squeezed against their edges.
/// - the glyph column, `edge` to `text`: where a chevron sits, and where
///   every glyph inside a box sits too, centered on the same x
///   (`glyphCenter`), so the pulsating dot lines up with the carets over it.
///
/// Nothing in a view carries a number of its own for any of these.
enum NavigatorGrid {
    /// The leading edge of the content column: column A.
    static let edge: CGFloat = ColumnGrid.a
    /// The glyph column's width, from `edge` to `text`: one step.
    static let mark: CGFloat = ColumnGrid.step
    /// The text column: column B.
    static let text: CGFloat = edge + mark
    /// How far into a row drawn from `edge` its text starts.
    static let textInset: CGFloat = text - edge
    /// How far a box reaches past `edge`, on each side: half the margin.
    static let outset: CGFloat = edge / 2
    /// Where a box starts, in the column's coordinates.
    static let boxEdge: CGFloat = edge - outset
    /// The x a chevron and every glyph in a box is centered on.
    static let glyphCenter: CGFloat = edge + mark / 2
}

/// The navigator's vertical rhythm (ov-243): the room over and under every
/// level, so the space says what belongs to what. The owner, 3 October: "the
/// gaps are inconsistent and don't make sense".
///
/// Every line in the navigator (a header's title, a row's text, "and 2
/// more") sits in a slot with `air` over and under it, so two slots that
/// touch read `2 * air` apart, the gap between two rows. A header sits that
/// close to the first thing it labels, as Finder's and Xcode's sidebars set
/// a section's title; the space between a header and what comes before it
/// grows with its level. What the eye sees, text to text:
///
/// | Level                                   | Above | Below |
/// |-----------------------------------------|-------|-------|
/// | Section header: Tasks, Terminals        |  24   |   8   |
/// | Group header: Unread, a task status     |  16   |   8   |
/// | Subgroup header: Finished, New          |  12   |   8   |
/// | Row, "and 2 more", New Terminal         |   8   |   —   |
/// | Rule under the orchestrator             |  12   |  12   |
///
/// A rule stands in the middle of the section gap it replaces. The slot
/// gaps below are what a view adds between slots to get those: the gap
/// above, less the two slots' air. `NavigatorRhythmTests` measures them.
enum NavigatorRhythm {
    /// Over and under every line, inside its slot: a row's padding, a
    /// header's room inside its hit target.
    static let air: CGFloat = Spacing.tight
    /// Between two lines of one row: a task's title and its status line,
    /// the orchestrator's name and what it's doing.
    static let lineGap: CGFloat = Spacing.tight / 2
    /// Between two rows' slots, and between a header's slot and its
    /// first child's: none, so they read `2 * air` apart.
    static let row: CGFloat = 0
    /// Over a subgroup header that follows another subgroup.
    static let subgroup: CGFloat = Spacing.tight
    /// Over a group header that follows another group.
    static let group: CGFloat = Spacing.group
    /// Over a section header that follows another section.
    static let section: CGFloat = Spacing.section
    /// Over and under a rule between sections: half the section gap each.
    static let rule: CGFloat = (section + 2 * air) / 2 - air
    /// The top band's margin: over the filter, and between the filter's box
    /// and the orchestrator's row.
    static let band: CGFloat = Spacing.group

    /// What the eye reads between two lines whose slots are `slotGap` apart.
    static func visible(_ slotGap: CGFloat) -> CGFloat { slotGap + 2 * air }
}

extension View {
    /// Put this glyph (or chevron) in the navigator's glyph column: a cell
    /// from `edge` to `text`, the glyph centered in it.
    func glyphColumn() -> some View {
        frame(width: NavigatorGrid.mark, alignment: .center)
    }

    /// Let a box's shape (a fill, a focus ring) reach `NavigatorGrid.outset`
    /// past the content it surrounds, on both sides.
    func boxOutset() -> some View {
        padding(.horizontal, -NavigatorGrid.outset)
    }
}

/// What a mark in a row is, for `GridGeometryTests`. `box` is the leading
/// edge of a filled shape: a field, a row's selection.
enum GridRole: String, Sendable {
    case chevron, icon, text, box
}

/// One mark a row reported: which row type, which kind of mark, and where it
/// is. Reported only under `gridProbing`, so the app itself pays nothing.
struct GridMark {
    let row: String
    let role: GridRole
    let bounds: Anchor<CGRect>
}

struct GridMarksKey: PreferenceKey {
    static let defaultValue: [GridMark] = []
    static func reduce(value: inout [GridMark], nextValue: () -> [GridMark]) {
        value += nextValue()
    }
}

private struct GridProbingKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// Whether rows report their marks: set by `GridGeometryTests` around the
    /// real row views, and never by the app.
    var gridProbing: Bool {
        get { self[GridProbingKey.self] }
        set { self[GridProbingKey.self] = newValue }
    }
}

private struct GridMarkModifier: ViewModifier {
    let row: String
    let role: GridRole
    @Environment(\.gridProbing) private var probing

    func body(content: Content) -> some View {
        if probing {
            // Added to the marks inside it, not set: a box's mark would
            // otherwise hide its own glyph's and text's.
            content.transformAnchorPreference(key: GridMarksKey.self, value: .bounds) {
                $0.append(GridMark(row: row, role: role, bounds: $1))
            }
        } else {
            content
        }
    }
}

extension View {
    /// Say that this view is `row`'s `role` mark — its chevron cell, its icon
    /// cell or its text — so a geometry test can read where it really landed.
    func gridMark(_ row: String, _ role: GridRole) -> some View {
        modifier(GridMarkModifier(row: row, role: role))
    }
}

/// The two spacings rows outside the columns still share, kept from the old
/// Fleet sidebar's column system when it went (ov-178): the gap between a
/// mark and its text, and an icon control's square.
enum SidebarGrid {
    /// Space between a marker and the text it belongs to.
    static let gap: CGFloat = 8

    /// A square tap target for an icon control, so every one of them occupies
    /// the same box whatever glyph is inside it.
    static let control: CGFloat = 24
}
