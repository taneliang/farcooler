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

/// What a mark in a row is, for `GridGeometryTests`.
enum GridRole: String, Sendable {
    case chevron, icon, text
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
            content.anchorPreference(key: GridMarksKey.self, value: .bounds) {
                [GridMark(row: row, role: role, bounds: $0)]
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

/// The sidebar's one column system.
///
/// This exists because the previous arrangement had four independent sources of
/// horizontal inset — the search field's own 14, the header's title rail, an 8
/// on the scroll content, and whatever a `Menu` decided to add — and every
/// attempt to line two things up meant guessing which of the four applied to
/// which view. Four rounds of that produced four wrong answers.
///
/// The rule now: **nothing in the sidebar sets its own horizontal padding.**
/// Every row is handed the same band by `SidebarRow`, and indentation inside
/// that band is expressed in columns, not in numbers a caller invents.
///
/// There were two of these. A second enum named `Grid` sat at the top of
/// `SidebarViews` claiming the same job in the same words; ten of its thirteen
/// members were unused, including an entire declared indent model, and the
/// three that were live duplicated values already here. Two single sources of
/// truth is none, so it is gone and its survivors are below.
enum SidebarGrid {
    /// The band's inset from the window edge: column A. One number, one
    /// place.
    static let edge: CGFloat = ColumnGrid.a

    /// The disclosure chevron's column, and therefore one indent level: one
    /// grid step.
    static let gutter: CGFloat = ColumnGrid.step

    /// How far `ContentView.sidebarRow` indents a row drawn at `depth`.
    static func indent(_ depth: Int) -> CGFloat { CGFloat(depth) * gutter }

    /// A workspace, worktree or terminal row's content inset inside its
    /// highlight, so its first column lands on `edge` at depth 0.
    static let rowInset: CGFloat = edge - highlightInset

    /// Where a workspace or worktree row drawn at `depth` puts its chevron,
    /// from the sidebar's edge; its text starts one `gutter` (the chevron's
    /// column) further in. What the rows lay out from, and what
    /// `SidebarColumnTests` reads.
    static func chevron(depth: Int) -> CGFloat { indent(depth) + highlightInset + rowInset }

    /// The width of a workspace or worktree row's chevron column, the
    /// frame its chevron is drawn in.
    static let chevronColumn: CGFloat = gutter

    /// Where a workspace or worktree row drawn at `depth` puts its glyph:
    /// the column after its chevron.
    static func glyph(depth: Int) -> CGFloat { chevron(depth: depth) + chevronColumn }

    /// The width of a row's glyph cell: one column.
    static let glyphColumn: CGFloat = gutter

    /// Where a workspace or worktree row drawn at `depth` starts its text,
    /// the column after its glyph.
    static func text(depth: Int) -> CGFloat { glyph(depth: depth) + glyphColumn }

    /// Space between a marker and the text it belongs to.
    static let gap: CGFloat = 8

    /// The gap between two trailing cells in a row — the diff counts and the
    /// text that ends before them.
    static let cellGap: CGFloat = 6

    /// The tight gap between a mark and the count it qualifies, "○ 1", so
    /// the two read as a pair and not as one glyph (ov-81 P13).
    static let markGap: CGFloat = 4

    /// How far a row's selection highlight sits inside the band.
    ///
    /// The highlight is narrower than the band so a selected row reads as a
    /// pill inside the column rather than a stripe across the whole sidebar.
    /// Its content then sits at `edge`, like everything else.
    static let highlightInset: CGFloat = 8

    /// A square tap target for an icon control, so every one of them occupies
    /// the same box whatever glyph is inside it.
    static let control: CGFloat = 24

    /// Breathing room inside primary worktree and terminal rows, inside the
    /// row's `ColumnGrid.rowHeight` or `twoLineRowHeight`.
    static let rowVerticalPadding: CGFloat = 4

    /// Hidden and Unclaimed headings, and the rows inside Hidden: secondary,
    /// so held to the one-line row height and no taller.
    static let headerVerticalPadding: CGFloat = 4

    /// The rows inside a Hidden group.
    static let secondaryRowVerticalPadding: CGFloat = 4

    /// A row's fill while the pointer is over it.
    static let hoverFill = Color.primary.opacity(0.045)

    /// Project labels separate groups, so the space before one is deliberately
    /// larger than the space after it. That makes each heading belong to the
    /// worktrees below instead of floating halfway between two projects.
    static let projectTopPadding: CGFloat = 2 * ColumnGrid.rhythm
    static let projectBottomPadding: CGFloat = 0
}

/// One row of the sidebar, in the band every other row uses.
///
/// `indent` is in LEVELS, not points: each is one `ColumnGrid` step, so a row
/// drawn at `indent` n starts at column n (0 is column A). A caller that wants to nudge something by three points is
/// a caller about to break the column, which is exactly how this got out of
/// alignment before.
struct SidebarRow<Content: View>: View {
    var indent: Int = 0
    @ViewBuilder var content: Content

    var body: some View {
        content
            .padding(.leading, SidebarGrid.edge + CGFloat(indent) * SidebarGrid.gutter)
            .padding(.trailing, SidebarGrid.edge)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// An icon button that opens a menu, with geometry this file controls.
///
/// Not a bare `Menu`. `.menuStyle(.borderlessButton)` insets its own label by an
/// amount that is invisible, asymmetric, and — on the leading side — clamps how
/// far a negative padding may claw back, so two `Menu`s given identical padding
/// in different containers still landed in different columns. A `Button` has no
/// such chrome, so a fixed square frame here means the glyph is where the frame
/// is, every time.
///
/// The menu itself is a real `NSMenu`, popped at the button: a popover of
/// buttons would have been easier and would not have looked like the rest of
/// macOS.
struct SidebarMenuButton: View {
    let systemImage: String
    let help: String
    let items: [SidebarMenuItem]

    /// A real `NSView` sitting exactly where the button is, so the menu can be
    /// popped in the button's own coordinate system.
    @State private var anchor = MenuAnchor()

    var body: some View {
        Button {
            present()
        } label: {
            Image(systemName: systemImage)
                .font(.system(size: 12, weight: .semibold))
                .frame(width: SidebarGrid.control, height: SidebarGrid.control)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(help)
        .background(MenuAnchorView(anchor: anchor))
    }

    private func present() {
        SidebarMenuItem.popUp(items, under: anchor)
    }
}

extension SidebarMenuItem {
    /// Pop `items` as a real `NSMenu` below `anchor`'s view, headed by
    /// `header` when there is one — a section header, as a context menu's
    /// `Section` draws it.
    @MainActor
    static func popUp(_ items: [SidebarMenuItem], header: String? = nil, under anchor: MenuAnchor) {
        let menu = NSMenu()
        if let header { menu.addItem(.sectionHeader(title: header)) }
        for item in items {
            if item.isSeparator {
                menu.addItem(.separator())
                continue
            }
            let entry = NSMenuItem(title: item.title, action: #selector(MenuInvoker.fire), keyEquivalent: "")
            let invoker = MenuInvoker(item.action)
            entry.target = invoker
            entry.representedObject = invoker
            entry.state = item.isChecked ? .on : .off
            menu.addItem(entry)
        }
        // Below the control, aligned to its leading edge — expressed in the
        // control's own bounds, which is the only frame of reference here that
        // cannot be misread.
        if let view = anchor.view, view.window != nil {
            menu.popUp(positioning: nil, at: NSPoint(x: 0, y: view.bounds.maxY + 4), in: view)
        } else {
            menu.popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
        }
    }
}

/// `NSMenuItem` wants a target and a selector, and a SwiftUI closure is
/// neither — so one tiny object bridges them, kept alive by the item that
/// points at it.
private final class MenuInvoker: NSObject {
    private let action: () -> Void
    init(_ action: @escaping () -> Void) { self.action = action }
    @objc func fire() { action() }
}

/// Somewhere to hang the button's `NSView` between drawing it and using it.
///
/// A class, not a captured frame: a `CGRect` from a `GeometryReader` is measured
/// in SwiftUI's coordinates — origin top-left, y growing downward — and
/// `NSView.convert(_:from: nil)` reads whatever it is handed as AppKit window
/// coordinates, origin bottom-left, y growing upward. The two are mirror images,
/// so a project header near the bottom of the sidebar opened its menu near the
/// top of the window, and only a control at the exact vertical middle would have
/// looked right. Popping the menu inside the view means there is no axis left to
/// get backwards.
final class MenuAnchor {
    fileprivate(set) weak var view: NSView?
}

struct MenuAnchorView: NSViewRepresentable {
    let anchor: MenuAnchor

    func makeNSView(context: Context) -> NSView {
        let view = FlippedAnchorView()
        anchor.view = view
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        anchor.view = nsView
    }

    /// Flipped so `bounds.maxY` means the bottom edge, as it reads. Invisible to
    /// the mouse, because it sits on top of the button it is measuring.
    private final class FlippedAnchorView: NSView {
        override var isFlipped: Bool { true }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
}

struct SidebarMenuItem {
    let title: String
    let action: () -> Void
    var isSeparator = false
    /// Drawn with a checkmark: the current choice, in a menu that picks one.
    var isChecked = false

    /// A computed property, not a stored one: a struct holding a closure is
    /// not `Sendable`, and a `static let` of one is a concurrency error under
    /// Swift 6 even though nothing here is shared.
    static var separator: SidebarMenuItem {
        SidebarMenuItem(title: "", action: {}, isSeparator: true)
    }
}
