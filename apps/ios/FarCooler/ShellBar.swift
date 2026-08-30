import SwiftUI

// The bar that IS the workspace, and the column it grows into.
//
// One bar at the bottom, the way Safari's bar is the tab. It carries the
// workspace's name and a ribbon of its tabs; swiping it sideways changes
// workspace, dragging it up unfurls those tabs as a column with the selection
// following the finger, and a tap holds that column open. The gesture lives in
// `ShellRootView` and the arithmetic in `AgentKit/ShellNavigation.swift` — this
// file draws, and decides nothing.
//
// **One glass surface at a time.** The column is not a panel floating near the
// bar; it is the bar, grown. `TerminalTabStrip.swift:119` is where this app
// learned the difference between a surface and a boundary — `glassEffect(in:)`
// draws behind its content without constraining it, so the shape has to be
// CLIPPED as well as backed, or a row scrolling out of the column carries on
// straight through the corner. Same radius-22 pair every floating surface here
// uses, and one `GlassSurface` (`AgentView.swift:2973`) around the whole
// column-plus-bar rather than one apiece: two pieces of glass composite
// independently and read as two objects, which is exactly what "the column is
// the bar" is not.

/// One tab's mark: what that tab is doing, in six points.
///
/// The four drawings are the mechanics doc's table, verbatim. What matters
/// most is the fifth row of that table — the CURRENT tab is an ELONGATED
/// version of its own state, never a solid "selected" pill. A pill would
/// replace the one thing the mark is for: the ribbon has to keep saying what
/// each tab is doing while it says which one you are on, and a workspace where
/// the current tab is the one you cannot read the state of is a workspace you
/// have to open the column to understand.
///
/// Elongation is therefore the only thing `isCurrent` changes. Same fill, same
/// stroke, same colour; 2.5 times as wide, with a capsule radius — which at
/// equal width and height is a circle, so one shape draws both.
struct ShellMarkView: View {
    let mark: ShellMark
    /// 6 in the bar's ribbon, 7 in the column, 5 on an overview card.
    let size: CGFloat
    var isCurrent: Bool = false

    var body: some View {
        shape
            .frame(width: isCurrent ? size * 2.5 : size, height: size)
            // The mark is drawn state, not a control and not a label: the
            // ribbon as a whole is what a screen reader is offered, from the
            // bar's own label.
            .accessibilityHidden(true)
    }

    @ViewBuilder
    private var shape: some View {
        switch mark {
        // Amber means an agent is waiting on a person, here and on the widget,
        // the Live Activity and the inbox — see `TerminalTabStrip`'s
        // `ChangesChip`, which refuses the colour for exactly this reason.
        case .needsYou:
            Capsule().fill(Color.orange)
        // Only ever a Diff tab. `ShellMark.unreadDiff` says why.
        case .unreadDiff:
            Capsule().strokeBorder(Color.cyan, lineWidth: 1.6)
        case .working:
            Capsule().strokeBorder(Color.white.opacity(0.6), lineWidth: 1.4)
        // Dashed, and drawn at the same weight as `working` rather than at a
        // different one: staleness is the AGE of the daemon's answer, not a
        // fifth state competing with it, so it modifies the ring rather than
        // replacing it.
        case .stale:
            Capsule().strokeBorder(
                Color.white.opacity(0.45),
                style: StrokeStyle(lineWidth: 1.4, dash: [1.6, 1.6]))
        }
    }
}

/// The workspace's tabs, in their fixed order, as marks.
///
/// Never re-sorted by activity: the ribbon is a map of the workspace, and a
/// map whose landmarks move is one you have to read every time instead of
/// remembering. `ShellWorkspace` makes the same point about the same order.
struct ShellRibbon: View {
    let tabs: [ShellTab]
    let current: Int
    var size: CGFloat = 6

    var body: some View {
        HStack(spacing: 5) {
            ForEach(Array(tabs.enumerated()), id: \.element.id) { index, tab in
                ShellMarkView(mark: tab.mark, size: size, isCurrent: index == current)
            }
        }
    }
}

/// The workspace's tabs as a column, unfurling upward out of the bar.
///
/// Tab 0 nearest the bar, which is why the rows are reversed: the first 34
/// points of lift reveal the row under your finger, and that row is
/// `columnSelection(up:)`'s answer for one step, which is tab 0. The web
/// prototype spells this `column-reverse`; in SwiftUI it is a bottom-anchored
/// stack, and the reveal is a `.frame(height:alignment: .bottom)` over it.
///
/// There is deliberately **no workspace row here.** The bar sitting directly
/// beneath already carries the name and the ribbon, and repeating it was
/// removed in review — a header on a column whose header is two points below
/// it is the same word twice.
struct ShellColumn: View {
    let tabs: [ShellTab]
    /// The row the finger is over, or the tab you are on when the column is
    /// held open by a tap. Nil while the lift is below `openMin`, which is the
    /// state where letting go costs nothing.
    let selection: Int?

    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(tabs.enumerated()).reversed(), id: \.element.id) { index, tab in
                row(tab, isSelected: index == selection)
            }
        }
    }

    private func row(_ tab: ShellTab, isSelected: Bool) -> some View {
        HStack(spacing: 10) {
            ShellMarkView(mark: tab.mark, size: 7, isCurrent: isSelected)
                // A fixed width so the titles line up whatever each row's mark
                // is doing: an elongated mark is 17.5 wide and a round one 7,
                // and text that moved sideways as the selection passed it
                // would read as the list rearranging itself.
                .frame(width: 18, alignment: .leading)
            Text(tab.title)
                .font(.system(size: 15))
                .lineLimit(1)
                .foregroundStyle(isSelected ? .primary : .secondary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, PaneMetrics.edge)
        .frame(height: ShellMetrics.rowHeight)
        .background(
            // Amber at 20%, and only under the row the finger is on. The same
            // colour the marks use, at the weight that says "here" rather than
            // "this needs you".
            isSelected ? Color.orange.opacity(0.2) : Color.clear)
    }
}

/// The bar itself: one workspace's name and ribbon, with its neighbours
/// waiting off both edges.
///
/// Three rail items in a plain `HStack`, translated — the same idea as the
/// content track and for the same reason: the neighbour has to be genuinely
/// drawn and moving before the swipe commits, or the workspace you are
/// swiping to only exists after you have already committed to it.
struct ShellBarRail: View {
    let fleet: ShellFleet
    let position: ShellPosition
    /// Which workspace sits off each edge, along whichever track the current
    /// gesture is walking. Nil where there is nothing there.
    let previous: Int?
    let next: Int?
    /// One rail item's width — the bar's width, not the page's.
    let railWidth: CGFloat
    /// How far the rail has been dragged. Zero whenever the swipe will not
    /// change workspace, which is what makes the bar hold still within one.
    let railX: CGFloat

    var body: some View {
        HStack(spacing: 0) {
            item(previous)
            item(position.workspace)
            item(next)
        }
        // Three items centred in one item's width puts the middle one on
        // screen and the other two exactly one width off each edge, which is
        // the `-RAIL_W + dx` the prototype writes out longhand.
        .frame(width: railWidth * 3)
        .offset(x: railX)
        .frame(width: railWidth, height: ShellMetrics.barRow)
    }

    @ViewBuilder
    private func item(_ index: Int?) -> some View {
        Group {
            if let index, fleet.workspaces.indices.contains(index) {
                let workspace = fleet.workspaces[index]
                // Ribbon, then name, then the server — the order the brief
                // states, and not an arbitrary one. The ribbon is the part
                // read at a glance and the part that is the same shape every
                // time, so it anchors the left edge where the eye already is;
                // the name is what varies and gets the room to truncate. A
                // ribbon pushed to the right edge moves with the length of the
                // name, which is the one thing a mark you are meant to learn
                // must never do.
                HStack(spacing: 12) {
                    ShellRibbon(
                        tabs: workspace.tabs,
                        current: index == position.workspace ? position.tab : -1)
                    Text(workspace.name)
                        .font(.system(size: 17, weight: .medium))
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: 8)
                    if let server = workspace.server {
                        // Mono, because it came off a machine. See the type
                        // rule in the brief: if it is mono, it is data.
                        Text(server)
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                .padding(.horizontal, PaneMetrics.edge)
            } else {
                Color.clear
            }
        }
        .frame(width: railWidth, height: ShellMetrics.barRow)
    }
}

/// The whole surface: the column above, the bar below, one piece of glass.
struct ShellBar: View {
    let fleet: ShellFleet
    let position: ShellPosition
    let previous: Int?
    let next: Int?
    let railWidth: CGFloat
    let railX: CGFloat
    /// How much of the column is showing, in points. `ShellGesture.columnHeight`
    /// is the only thing that computes this — a pinned column and a dragged one
    /// are separate inputs there and must stay separate here.
    let columnHeight: CGFloat
    let columnSelection: Int?
    /// The column dissolves as the overview arrives, while the bar only fades.
    let columnOpacity: CGFloat

    var body: some View {
        VStack(spacing: 0) {
            ShellColumn(tabs: workspace?.tabs ?? [], selection: columnSelection)
                // Bottom alignment is the unfurl: the frame grows upward out of
                // the bar and reveals the rows nearest it first.
                .frame(height: max(0, columnHeight), alignment: .bottom)
                .clipped()
                .opacity(columnOpacity)

            ShellBarRail(
                fleet: fleet, position: position, previous: previous, next: next,
                railWidth: railWidth, railX: railX)
        }
        .frame(width: railWidth)
        .clipShape(RoundedRectangle(cornerRadius: PaneMetrics.surfaceRadius))
        .modifier(GlassSurface(radius: PaneMetrics.surfaceRadius))
        // The whole surface is the target, not the glyphs on it.
        //
        // Without this the bar is only touchable where something is DRAWN:
        // hit testing walks the content, and a `VStack` holding a name at one
        // end and a ribbon at the other is mostly gap. A tap in the middle of
        // the bar — which is the middle of the bar — landed on nothing at all,
        // and the gesture `ShellRootView` attaches out here never saw a touch.
        // The same rule `PaneMetrics.target` describes for every control in
        // this app: the visible thing keeps its size, and a shape around it
        // makes the band between them live rather than merely occupied.
        .contentShape(.rect)
        // One label for the whole surface rather than an element per mark: the
        // ribbon is a picture of the workspace, and read out dot by dot it is
        // forty words that say nothing.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(workspace.map { "Workspace \($0.name)" } ?? "No workspace")
        .accessibilityIdentifier("shell-bar")
    }

    private var workspace: ShellWorkspace? {
        fleet.workspaces.indices.contains(position.workspace)
            ? fleet.workspaces[position.workspace] : nil
    }
}
