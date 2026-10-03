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

/// The navigator's grid: two lines down the board column, and nothing
/// between them (ov-177, round 2).
///
/// - `edge`: where everything drawn full width starts. A section's chevron,
///   the filter field's box, and a row's selection (the orchestrator's
///   included) all start here.
/// - `text`: where every word starts. The header's title, a section's and a
///   group's title, a task's key, the filter's text and the orchestrator's
///   title all start here.
///
/// A mark (chevron, filter glyph, orchestrator status) sits in the one cell
/// between the two lines. The owner's screenshot of 3 October showed the
/// filter's text and the orchestrator's title a column further in, at C,
/// with their glyphs on the rows' text column. Their boxes looked indented
/// as a result.
enum NavigatorGrid {
    /// The leading edge of full-width things: column A.
    static let edge: CGFloat = ColumnGrid.a
    /// The mark cell, from `edge` to `text`: one step.
    static let mark: CGFloat = ColumnGrid.step
    /// The text column: column B.
    static let text: CGFloat = edge + mark
    /// How far into a box drawn from `edge` its text starts.
    static let textInset: CGFloat = text - edge
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
