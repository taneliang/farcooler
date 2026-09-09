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
// bar; it is the bar, grown. The tab strip this replaced is where this app
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
/// stroke, same color; 2.5 times as wide, with a capsule radius — which at
/// equal width and height is a circle, so one shape draws both.
///
/// **The four drawings are gone; this is `GlanceMarkView` now.** They were four
/// color literals — `Color.orange`, `Color.cyan`, and white at two opacities —
/// which made this file the third of three places one amber could drift, beside
/// the two `glanceTint` functions in the widget extensions. The glance spec's
/// §03 is one mark for the whole product, and the argument for it is exactly
/// the argument this file already makes about the ribbon: a mark is worth
/// having only if it is the same mark everywhere, or it is something to be read
/// rather than recognised. `GlanceMark` carries the states; this view carries
/// the ribbon's own size and its elongation.
///
/// **And the tab now HOLDS a `GlanceMark` rather than being translated into
/// one here.** There was a `ShellMark` enum in front of this — four cases, a
/// second vocabulary for the same thing — and a bridging initializer in this
/// file that turned one into the other. It is gone; `ShellTab.mark` says why at
/// length. The short version is that the bridge could not carry what it was
/// given: `ShellMark` had one case for a producing agent and an idle one, so a
/// working agent drew the same hairline as an agent sitting at a prompt, and
/// the bar said nothing at all while a turn ran.
struct ShellMarkView: View {
    let mark: GlanceMark
    /// 6 in the bar's ribbon, 7 in the column, 5 on an overview card.
    ///
    /// **Smaller than any of §03's six diameters, and deliberately kept.** The
    /// spec's ladder starts at an 8pt ribbon, but these three numbers are
    /// load-bearing in layout as well as in drawing: `ShellColumn` reserves an
    /// 18pt gutter around a 7pt mark, and the flight between the ribbon and the
    /// menu is a `matchedGeometryEffect` whose source frames are literally
    /// `size` and `size * 2.5`. Growing them to satisfy a spec about GLANCE
    /// surfaces — widgets, the lock screen, the wrist — would move three frames
    /// in the app, where the person is already looking. What this ribbon takes
    /// from §03 is the drawing, which is what was drifting.
    ///
    /// These sizes have no room for §03's concentric core, and
    /// `GlanceMarkView.init(_:inAppDiameter:elongated:)` is where what they do
    /// instead — say `producing` with fill — is argued.
    let size: CGFloat
    var isCurrent: Bool = false

    var body: some View {
        // Decorative, and hidden, because the ribbon as a whole is what a
        // screen reader is offered from the bar's own label. `GlanceMark.phrase`
        // is what says the tier in words on the surfaces where the mark stands
        // alone — §06 of the visual brief, since stroke weight reaches VoiceOver
        // through nothing else.
        GlanceMarkView(mark, inAppDiameter: size, elongated: isCurrent)
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
    /// The namespace this shell's marks live in, when there is one.
    ///
    /// **There is exactly one dot per tab, and it is in two places.** The
    /// ribbon and the menu were two independently drawn sets of the same
    /// information, and a person watching the menu open saw one set of dots
    /// appear while another set stayed where it was — which says these are two
    /// pictures of a workspace rather than one workspace with its agents in
    /// it. A dot IS the agent, so opening the menu has to be the dots moving.
    ///
    /// What travels here is a frame, which is the one thing
    /// `matchedGeometryEffect` does and the reason it is right here and was
    /// wrong for the page: a page carries a terminal and matching its frame
    /// re-lays-out that terminal at every size on the way down, forty times a
    /// second. A mark is a capsule six points across with no contents at all.
    /// There is nothing inside it to reflow.
    ///
    /// Nil for a ribbon that is only ever in one place — the overview card's,
    /// which draws the same marks at 5 points and has no menu to fly to.
    var marks: Namespace.ID?
    /// Whether the menu is open, and therefore whether the ribbon's slots or
    /// the menu's rows are the ones saying where the dots ARE.
    var menuOpen = false
    /// Which dot is drawn elongated while the menu is open — the row under the
    /// finger, rather than the tab you are still on.
    ///
    /// Separate from `current`, and the split is between two different jobs
    /// one number was doing. `current` sizes the SLOTS, which is layout: the
    /// ribbon has to reserve the same space whichever row a finger is over, or
    /// the workspace's name would shuffle sideways as the selection walked the
    /// menu.
    ///
    /// **Unused now, and kept only so the two slot questions stay separable.**
    /// It used to elongate the row under the finger, and that cost the mark
    /// its job: the capsule is the only thing that says which terminal you are
    /// ON, so moving it to whatever you are hovering left nothing at all
    /// saying where you actually are. Hovering already has its own signal —
    /// the row's selection fill — and the two are different facts. The
    /// elongation stays on `current`.
    var menuMark: Int?

    /// Which mark is drawn elongated, which lags `current` by exactly one
    /// animation.
    ///
    /// **A stored copy rather than `current` itself, and that is the whole of
    /// why the mark travels.** The elongation is a width — the same mark, 2.5
    /// times as wide — so a ribbon whose widths interpolate is a ribbon where
    /// the long mark appears to slide along, pushing its neighbours aside and
    /// closing up behind itself. Every ingredient for that was already here
    /// and none of it ever ran, because of WHERE the selection changes: a tab
    /// swipe re-seats `position` inside `ShellRootView.commit`'s deliberately
    /// SILENT transaction, and a silent transaction is silent for everything
    /// downstream of it. The ribbon cut because the commit that moved it was
    /// required to be invisible for an unrelated reason — the track's
    /// no-bounce re-seat.
    ///
    /// An `onChange` runs after that update rather than inside it, so the
    /// `withAnimation` below opens a transaction of its own that the silent
    /// one cannot reach. The cost is one frame of lag, which is exactly the
    /// frame in which the pane has already arrived and the ribbon has not yet
    /// started moving — and that is what it should look like: the mark
    /// follows you to the tab, it does not arrive before you.
    ///
    /// **That one frame of lag is only tolerable while it is a lag about the
    /// SAME workspace**, which is what `shownFor` is for. See `elongated`.
    @State private var shown: Int = -1
    /// The tabs `shown` is an index into.
    ///
    /// Without this, the lag above becomes a lie the moment a crossing swaps
    /// the whole tab set underneath the bar. This view is at a fixed place in
    /// the bar track — the middle of three — so a crossing does not rebuild
    /// it; it hands the same `ShellRibbon` a different workspace's `tabs` and
    /// a different `current`, and `shown` goes on holding an index into a
    /// workspace that is no longer on screen. For the one frame before
    /// `onChange` catches up, the elongated capsule sits on whichever tab of
    /// the ARRIVING workspace happens to share that index — a tab that has
    /// never been open, in a workspace you have just this instant reached.
    /// That is the flash, and the animation then makes it worse by sliding
    /// the mark from the wrong tab to the right one, which reads as having
    /// been on the wrong one.
    ///
    /// The ids rather than a count or the workspace's own id: two workspaces
    /// can have the same number of tabs, and the ribbon is about the tabs.
    @State private var shownFor: [String] = []

    /// Which mark is drawn elongated THIS frame.
    ///
    /// `shown` while it is still an index into the tabs on screen — that is
    /// the deliberate one-frame lag that makes the capsule travel — and
    /// `current` the instant it is not. Derived rather than corrected in a
    /// change handler, because a change handler runs after the frame is drawn
    /// and the frame that would be wrong is the first one.
    private var elongated: Int {
        shownFor == tabIDs ? shown : current
    }

    private var tabIDs: [String] { tabs.map(\.id) }

    var body: some View {
        HStack(spacing: 5) {
            ForEach(Array(tabs.enumerated()), id: \.element.id) { index, tab in
                // `elongated` for both: the capsule marks the tab you are ON,
                // never the row you are pointing at. See `menuMark`.
                slot(tab, holds: index == elongated, marked: index == elongated)
            }
        }
        // Seeded, not animated: the bar you launch onto is already on a tab,
        // and a mark that grew into place on first appearance would be the
        // shell announcing a move nobody made.
        .onAppear { seat() }
        .onChange(of: current) { _, moved in
            // Only where the tabs are the tabs `shown` belongs to. Crossing a
            // workspace changes `current` too, and animating THAT would be the
            // capsule sliding between two workspaces' tabs as though it had
            // walked from one to the other.
            guard shownFor == tabIDs else { return seat() }
            withAnimation(ShellMotion.ribbon) { shown = moved }
        }
        // A workspace arriving under this bar re-seats rather than travels.
        // Unanimated, because nothing moved: a different workspace's ribbon is
        // a different set of dots, and a capsule that slid into place across
        // that change would be claiming a journey between two tabs that have
        // never been on screen together.
        .onChange(of: tabIDs) { _, _ in seat() }
    }

    /// Put the capsule on the current tab, at once and without animation.
    private func seat() {
        shown = current
        shownFor = tabIDs
    }

    /// One tab's place in the ribbon, and — while the menu is shut — the dot
    /// standing in it.
    ///
    /// Three views carry this tab's id: this slot, the matching slot in the
    /// menu row, and the dot itself. Exactly one slot is the SOURCE at a time
    /// — this one while the menu is shut, the row's while it is open — and
    /// the dot is never a source, so it is always drawn wherever the live
    /// slot is. Switching which slot is the source inside an animated
    /// transaction is what makes the dot fly.
    ///
    /// **The dot is drawn HERE, in the bar, and never in the row.** The menu
    /// is a window that is clipped to its own height, and it starts that
    /// height at zero: a dot drawn inside it would be cut off for the whole
    /// first half of the flight up. The bar row is not clipped by anything
    /// but the surface itself, which covers the menu too, so a dot that lives
    /// in the bar can be sent anywhere on the glass and stay visible.
    ///
    /// The slot keeps its space either way, which is why it is a `Color.clear`
    /// with a frame rather than an `if`. A ribbon whose dots left the layout
    /// as well as the screen would slide the workspace's name sideways every
    /// time the menu opened.
    @ViewBuilder
    private func slot(_ tab: ShellTab, holds: Bool, marked: Bool) -> some View {
        if let marks {
            Color.clear
                .frame(width: holds ? size * 2.5 : size, height: size)
                .matchedGeometryEffect(id: tab.id, in: marks, isSource: !menuOpen)
                .overlay {
                    ShellMarkView(mark: tab.mark, size: size, isCurrent: marked)
                        .matchedGeometryEffect(id: tab.id, in: marks, isSource: false)
                }
        } else {
            ShellMarkView(mark: tab.mark, size: size, isCurrent: marked)
        }
    }
}

/// The workspace's tabs as a column, unfurling upward out of the bar.
///
/// **Tab 0 at the TOP, in the ribbon's own order.** A menu reads top to
/// bottom, and this one has a ribbon of the same tabs two points below it
/// whose leftmost mark is tab 0 — so the leftmost mark and the topmost row
/// have to be the same tab or the two halves of one bar disagree about which
/// end a workspace starts at.
///
/// The rows used to be reversed, putting tab 0 nearest the bar so that the
/// first `rowHeight` of lift selected it. That is the same decision as this
/// one seen from the other side, and inverting the drawing inverts the
/// mapping: the row nearest the bar is now the LAST tab, so
/// `ShellGesture.columnRow` counts DOWN from `tabCount` and a finger high
/// enough to be at the top of the column is on tab 0. The mapping lives in the
/// pure model where a test can hold it; this file only draws.
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
    /// The tab actually being shown, which is a different fact from the row a
    /// finger is over. The capsule marks this one; the selection fill marks
    /// the other. Conflating them left nothing on screen saying where you are.
    var current: Int = -1
    /// The namespace the shell's marks live in. See `ShellRibbon.marks`.
    var marks: Namespace.ID?
    /// Whether these rows are the ones saying where the dots are.
    var menuOpen = false

    /// Whether a row may be swiped to reveal Close.
    ///
    /// **The pinned column only, and that is the owner's third constraint
    /// verbatim**: *"if the user swipes up from the workspace bar, they should
    /// not be able to delete terminals that way"*. A dragged column is the
    /// middle of a navigation gesture — a thumb travelling up the phone with
    /// the page in it — and putting a destructive action inside it would mean
    /// the same continuous movement chooses a tab or destroys one depending on
    /// which way it curved. Pinning is a separate, completed act: a tap, with
    /// the finger already lifted, on a menu that is standing still.
    ///
    /// `ShellRootView.columnPinned` is the fact, and it is the fact rather than
    /// a second reading of one: the shell already keeps a pinned column and a
    /// dragged one as separate inputs — that is `ShellGesture.columnHeight`'s
    /// whole signature — so this gate is that existing distinction spent, not a
    /// new threshold that could disagree with it.
    var closable = false
    /// A row's Close, tapped. Nil in a bar that has no runner behind it — the
    /// two neighbours on the track, and the harness's fixtures.
    var onClose: ((ShellTab) -> Void)?
    /// Which row a finger is resting on, reported by the row itself, and nil
    /// the moment it lifts.
    ///
    /// **This is what the `List` cost, and it had to be given back.** A pinned
    /// column's touch-down highlight used to come from the bar's own
    /// `DragGesture` — `ShellDrag.barMoved`'s no-axis arm writes `fingerAbove`
    /// on the very first frame of a touch, and `ShellRootView.columnSelection`
    /// turns that into the lit row. A `List` cell arbitrates that touch away:
    /// the tap still reaches the bar at the END (`.land` fires and the tab
    /// changes), but the frames BEFORE the release do not, so the row under the
    /// thumb stopped lighting up at all. Measured after the rebuild:
    /// 0.98 brightness levels between the thumbed row and its neighbour,
    /// against the 8 the assertion asks for and the 24.74 a lit row stands at.
    ///
    /// That is not cosmetic and it is not new ground — it is the defect
    /// `ShellGestureTests.testAColumnRowLightsUpUnderAThumb` exists for, and
    /// WWDC 2018 803 puts it first among the things a tap owes: *"the button
    /// should highlight immediately when I touch down on it… but we shouldn't
    /// confirm the tap until my touch goes up."*
    ///
    /// A `simultaneousGesture` and not a `gesture`: the row still has to reach
    /// the swipe recognizer underneath it and the bar's release above it, and
    /// this reports rather than claims. It is a SECOND answer to "which row is
    /// that", which this file otherwise refuses — but it is the row's own
    /// index, which cannot be off by one, and it only ever applies where the
    /// geometric mapping has gone silent. The DRAGGED column keeps
    /// `ShellGesture.columnRow` exactly as it was: that touch starts on the bar
    /// row, below this list, so nothing arbitrates it away.
    var onTouch: ((Int?) -> Void)?
    /// A row, chosen. Its own index, which is the tab.
    ///
    /// **The rows are BUTTONS now, and that is the other half of what the list
    /// cost.** A pinned column's tap used to be answered by the bar's own
    /// gesture and resolved through `ShellGesture.columnRow` — the geometry
    /// that also answers the drag — and a list cell arbitrates that away in
    /// both directions: reporting the press from a `simultaneousGesture`
    /// restored the highlight and took the tap with it, because a
    /// `DragGesture(minimumDistance: 0)` inside a cell claims the touch
    /// outright and neither the swipe nor the bar's release saw another frame.
    /// Measured: the highlight came back, `testTappingAColumnRowSwitchesToThatTab`
    /// went red, and so did both swipe tests.
    ///
    /// A `Button` is the platform's own answer to the same pair. It highlights
    /// on touch-down through `ButtonStyle.isPressed`, it confirms on touch-up,
    /// and it yields to the swipe recognizer the moment a drag begins — which
    /// is three behaviors that were being hand-built out of one drag gesture.
    ///
    /// It is a second route to `.land`, and the file's own rule against two
    /// mappings is why this is the row's INDEX rather than a second reading of
    /// the geometry: an index cannot be off by one against itself. The DRAGGED
    /// column is untouched and still lands through `ShellFleet.barRelease` —
    /// that touch begins on the bar row, below this list, so no cell ever sees
    /// it.
    var onChoose: ((Int) -> Void)?

    var body: some View {
        // A `List`, and it is the swipe that requires one.
        //
        // `.swipeActions` is a `List` modifier and has no equivalent anywhere
        // else in SwiftUI; the owner asked for *"the native uitableview swipe
        // to delete interaction"* by name, so a hand-rolled offset-and-reveal
        // is out — it would be a drawing of the gesture rather than the
        // gesture, and it would arrive with none of the rubber-banding, the
        // velocity, the full-swipe threshold or the VoiceOver actions the real
        // one carries.
        //
        // **This is the surface the file's own header warns about**, so read
        // `ShellBar.columnWindow` before touching it. The rows must be laid out
        // EXACTLY ONCE or the menu scrambles as it opens, and what guarantees
        // that is unchanged: the fixed frame around this list absorbs the
        // window's animating proposal, so nothing this list contains ever hears
        // that a height is moving. A `List` is if anything a better citizen
        // there than the `VStack` was — its rows are laid out by UIKit inside a
        // scroll view whose bounds are pinned by that frame — but the property
        // was verified rather than reasoned about: `ShellColumnMotionTests`
        // drives the open with the spring slowed and reads the row positions
        // out of the frames, and the trace is in the lane's report.
        //
        // Everything below is the list being told to stop being a list. It has
        // no scroll of its own (the frame is exactly the content), no
        // background (the one piece of glass is outside it), no separators, no
        // insets and no row height but the shell's own — because the column is
        // not a table that happens to sit in a bar, it is the bar, grown, and
        // a hairline or an inset would be the platform drawing a second
        // surface inside the only one this file is allowed to have.
        List {
            ForEach(Array(tabs.enumerated()), id: \.element.id) { index, tab in
                Button {
                    onChoose?(index)
                } label: {
                    row(tab, isSelected: index == selection, isCurrent: index == current)
                }
                .buttonStyle(ShellRowPress { onTouch?($0 ? index : nil) })
                    // Named by its PLACE, because that is what a test presses.
                    // The title is on the button's label already and is the
                    // fixture's, not the shell's — two rows in one fleet can
                    // and do carry the same agent's name.
                    .accessibilityIdentifier("shell-column-row-\(index)")

                    .listRowInsets(EdgeInsets())
                    .listRowSeparator(.hidden)
                    .listRowBackground(Color.clear)
                    .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                        // **No full swipe, deliberately.** `allowsFullSwipe`
                        // makes a long enough flick fire the first action
                        // without the button ever being pressed, and the whole
                        // reason this lives behind a pinned menu is that the
                        // owner does not want a terminal closed by a movement.
                        // The swipe reveals; the tap decides.
                        //
                        // Absent rather than disabled on the Diff tab. The
                        // Mac's rule, kept: a daemon-side refusal is a safety
                        // net, and the button should not be there to press.
                        if closable, tab.closable {
                            Button(role: .destructive) { onClose?(tab) } label: {
                                Label("Close", systemImage: "trash")
                            }
                        }
                    }
            }
        }
        .listStyle(.plain)
        // The list is exactly as tall as its rows, so there is nothing to
        // scroll and a scroll would be a way to move the menu's contents
        // inside a menu that is itself opening. It also keeps the list's pan
        // recognizer off a surface whose whole job is to be dragged: the bar
        // owns the vertical gesture, and it starts on the bar row below this.
        .scrollDisabled(true)
        // Both of these, and they are two different backgrounds. The scroll
        // content's is the system grouped fill a `List` paints behind its rows;
        // the safe-area one is what it paints under them when the list is
        // shorter than its container. Either left in place is an opaque
        // rectangle inside the glass — see the file header: the column is one
        // surface, and a `List` arrives believing it is a screen.
        .scrollContentBackground(.hidden)
        .background(Color.clear)
        // A plain list still reserves a little room top and bottom for a
        // section that is not there. Zeroed, because `fullColumnHeight` is
        // `tabs.count * rowHeight` exactly and every point of inset is a point
        // of row pushed out through the clip.
        .contentMargins(.vertical, 0, for: .scrollContent)
        .environment(\.defaultMinListRowHeight, ShellMetrics.rowHeight)
    }

    /// One tab's place in the menu.
    ///
    /// A SLOT and not a dot — the dot itself is drawn down in the ribbon and
    /// sent here, for the reason `ShellRibbon.slot` gives — except on a bar
    /// with no namespace, where there is nothing to share with and the row
    /// draws its own.
    ///
    /// Sized to the mark rather than to the 18-point column the titles line up
    /// against, because the source's frame is what the dot is drawn at: a slot
    /// the width of the whole gutter would stretch a six-point capsule into an
    /// eighteen-point one on arrival.
    @ViewBuilder
    private func mark(_ tab: ShellTab, isCurrent: Bool) -> some View {
        if let marks {
            Color.clear
                .frame(width: isCurrent ? 7 * 2.5 : 7, height: 7)
                .matchedGeometryEffect(id: tab.id, in: marks, isSource: menuOpen)
        } else {
            ShellMarkView(mark: tab.mark, size: 7, isCurrent: isCurrent)
        }
    }

    private func row(_ tab: ShellTab, isSelected: Bool, isCurrent: Bool) -> some View {
        HStack(spacing: 10) {
            mark(tab, isCurrent: isCurrent)
                // A fixed width so the titles line up whatever each row's mark
                // is doing: an elongated mark is 17.5 wide and a round one 7,
                // and text that moved sideways as the selection passed it
                // would read as the list rearranging itself.
                .frame(width: 18, alignment: .leading)
            Text(tab.title)
                // 17, the body size a menu row uses. The rows read as a
                // system menu rather than as a bespoke list, which is what
                // they are.
                .font(.system(size: 17))
                .lineLimit(1)
                .foregroundStyle(isSelected ? .primary : .secondary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, PaneMetrics.edge)
        .frame(maxWidth: .infinity, minHeight: ShellMetrics.rowHeight, alignment: .leading)
        // The whole row is the swipe's target and the tap's, not the words on
        // it. A `List` row hit-tests its content, and a row holding a mark at
        // one end and a short name at the other is mostly gap — the same rule
        // `ShellBar` states for the bar itself.
        .contentShape(.rect)
        .background(
            // The platform's own selection fill, NOT amber.
            //
            // The prototype highlights the row under the finger in amber at
            // 20%, and that cannot be right here: "amber is reserved —
            // `#FF9F0A` means exactly one thing, an agent has stopped and
            // needs a human. Nothing else in the product may be amber." A row
            // being under your thumb is not an agent needing you, and a
            // color that means two things means neither.
            //
            // (The same contradiction sits in the brief's own §4, which
            // outlines the current workspace's card in amber. Flagged rather
            // than silently resolved.)
            isSelected ? AnyShapeStyle(.fill.tertiary) : AnyShapeStyle(.clear))
    }
}

/// A column row's press, drawn by the row and reported to the shell.
///
/// The whole of what a `ButtonStyle` is for here is `isPressed`: the platform
/// already knows when a finger is down on a control and when it has slid off,
/// and it knows it a frame before any gesture of ours could. What this adds is
/// telling the shell — `ShellRootView.columnSelection` decides which row is
/// lit, because the same question has to be answered for a dragged column,
/// where there is no button under the finger at all.
///
/// `onChange` rather than calling out of `makeBody` directly. A style's body is
/// evaluated during a view update, and writing state from inside one is the
/// modification-during-update SwiftUI warns about and then resolves by
/// dropping a frame.
private struct ShellRowPress: ButtonStyle {
    let onPress: (Bool) -> Void

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .onChange(of: configuration.isPressed) { _, pressed in onPress(pressed) }
    }
}

/// One workspace's bar, and the column of its tabs, as ONE piece of glass.
///
/// **One surface, morphed — never two stacked.** The column used to be its own
/// sheet floating a few points above the bar, and that is glass on glass: "Always
/// avoid glass on glass. Stacking Liquid Glass elements can quickly make the
/// interface feel cluttered." Glass belongs to the functional layer — controls,
/// navigation, transient UI — and two pieces of it a few points apart composite
/// independently, sample the background separately and read as two objects. The
/// column is not a second object. It is this bar, grown.
///
/// So there is exactly one `glassEffect` here, over a stack whose HEIGHT is the
/// only thing that changes, inside a `GlassEffectContainer` and carrying a
/// `glassEffectID` — which is what tells the platform that the short shape and
/// the tall shape are the same surface at two moments rather than one surface
/// replacing another. The bar morphs into the menu.
///
/// ## Why the contents cannot be inside the thing that grows
///
/// A surface whose SIZE animates has its contents laid out at every intermediate
/// size, so a column that grew by growing a stack of rows re-flowed those rows on
/// every frame of the spring — each row's text and mark animating its own
/// position independently, the whole list scrambling while the panel opened. Two
/// frames, a clip, and a window over a fixed-size child were all tried against
/// the symptom; none of them addressed the cause, and the fix that finally
/// stuck — giving the column its own sheet of glass — traded the bug for the
/// glass-on-glass this file now refuses.
///
/// The answer that is both: the rows are laid out ONCE, at the column's natural
/// height, by a `frame` with both dimensions fixed. A fixed frame absorbs the
/// proposal, so nothing the animation proposes ever reaches a row. What animates
/// is a shorter frame around that fixed one, bottom-aligned and clipped — a
/// WINDOW that opens over already-laid-out content. The glass shape follows the
/// window; the rows never move relative to each other, because nothing ever asks
/// them to lay out again.
struct ShellBar: View {
    /// The workspace this bar is. Nil off the ends of the fleet, where the
    /// slot exists so the track keeps its three positions but has nothing to
    /// draw.
    let workspace: ShellWorkspace?
    /// Which of its tabs is current, or -1 on a bar that is not the one you
    /// are in — a neighbour sliding past must not claim a current tab it does
    /// not have.
    let currentTab: Int
    /// The bar's width. The page's, less `ShellMetrics.barInset` each side.
    let width: CGFloat
    /// How much of the column is showing, in points. `ShellGesture.columnHeight`
    /// is the only thing that computes this — a pinned column and a dragged one
    /// are separate inputs there and must stay separate here.
    ///
    /// **A STEP, and never a value that tracks a finger.** It is whole or it is
    /// nothing, at both ends: the menu springs open at `openMin` and it pops
    /// shut the moment the lift goes past the last row. Nothing in between is
    /// ever passed in, which is what lets the animation below be a spring
    /// rather than a follow — see `ShellMotion.menu`.
    var columnHeight: CGFloat = 0
    var columnSelection: Int? = nil
    /// Whether this bar's column is the PINNED one, and therefore whether its
    /// rows may be swiped to close a terminal.
    ///
    /// Passed straight through to `ShellColumn.closable`, which carries the
    /// argument. It defaults to false, so the two neighbour bars on the track —
    /// which are drawn a page off the screen and are not hit-testable at all —
    /// cannot offer it either.
    var columnPinned = false
    /// A row's Close, tapped. See `ShellColumn.onClose`.
    var onClose: ((ShellTab) -> Void)?
    /// The row a finger is resting on. See `ShellColumn.onTouch`.
    var onTouch: ((Int?) -> Void)?
    /// A row, chosen. See `ShellColumn.onChoose`.
    var onChoose: ((Int) -> Void)?
    /// Whether this bar can grow a column at all.
    ///
    /// **False for the two neighbours on the track, and it is not an
    /// optimization.** A neighbour is a page off the display, is not
    /// hit-testable and has `columnHeight` nailed to zero, so its column is
    /// three rows nobody can see — and since the rebuild those rows are
    /// `List` cells, which are hosted as accessibility elements of their own
    /// and leak straight through the `accessibilityHidden(true)`
    /// `ShellRootView.neighbourBar` puts on them and through `ShellBar`'s own
    /// `accessibilityElement(children: .ignore)`. Measured: every row
    /// identifier matched three times, and XCUITest refused to press any of
    /// them because none of the three was the one.
    ///
    /// So a bar that cannot open a menu does not build one. What that costs is
    /// nothing on screen — a neighbour's column window is already zero points
    /// tall at zero opacity — and what it saves is two whole lists composed on
    /// every frame of every workspace swipe.
    var showsColumn = true

    /// The identity the glass keeps across the morph.
    ///
    /// One namespace per bar rather than one shared across the three on the
    /// track, because the three are three different surfaces on three
    /// different screens; giving them one identity would ask the platform to
    /// morph a workspace's bar into its neighbour's as they slide past.
    @Namespace private var glass

    /// The namespace the workspace's dots share between the ribbon and the
    /// menu. One per bar, like the glass's, and for the same reason: the
    /// three bars on the track are three different screens, and one namespace
    /// across them would ask a workspace's dot to fly into its neighbour's.
    @Namespace private var marks

    private var tabs: [ShellTab] { workspace?.tabs ?? [] }

    /// The column at its natural size — the size its rows are laid out at, and
    /// the only size they are ever laid out at.
    private var fullColumnHeight: CGFloat {
        CGFloat(tabs.count) * ShellMetrics.rowHeight
    }

    var body: some View {
        GlassEffectContainer(spacing: 0) {
            VStack(spacing: 0) {
                columnWindow
                barRow
            }
            .frame(width: width)
            // CLIPPED as well as backed. The tab strip this replaced is where
            // this app learned the difference: `glassEffect(in:)` draws a
            // surface behind its content and does not constrain it, so a row
            // at the top of the column carried on straight through the rounded
            // corner. A background is not a boundary.
            .clipShape(RoundedRectangle(cornerRadius: PaneMetrics.surfaceRadius))
            // INTERACTIVE glass, because this surface is the control.
            //
            // The bar is the one thing on this screen you put a finger ON and
            // move — sideways for the next workspace, up for the column — and
            // without this it was the only draggable thing in the app that did
            // not acknowledge being touched. iOS 26 draws that acknowledgement
            // itself, at the render layer and at the point of contact: the
            // glass scales, springs and lights up under the finger, on the
            // display's own refresh rather than on anything this file
            // animates. It is a finish on the touch, not a motion — none of
            // the flight, the crossing or the lift changes by a point.
            //
            // Only here. The cards in the overview are CONTENT, and content
            // does not get glass at all, interactive or otherwise: "always
            // avoid glass on glass", and a grid of forty flexing rectangles
            // would be the same mistake this file already corrected once when
            // the column was its own sheet.
            .modifier(GlassSurface(radius: PaneMetrics.surfaceRadius, interactive: true))
            .glassEffectID(Self.surfaceID, in: glass)
        }
        // The whole surface is the target, not the glyphs on it.
        //
        // Without this the bar is only touchable where something is DRAWN:
        // hit testing walks the content, and a row holding a name at one end
        // and a ribbon at the other is mostly gap. A tap in the middle of the
        // bar — which is the middle of the bar — landed on nothing at all, and
        // the gesture `ShellRootView` attaches out here never saw a touch. The
        // same rule `PaneMetrics.target` describes for every control in this
        // app: the visible thing keeps its size, and a shape around it makes
        // the band between them live rather than merely occupied.
        .contentShape(.rect)
        // One label for the whole surface rather than an element per mark: the
        // ribbon is a picture of the workspace, and read out dot by dot it is
        // forty words that say nothing.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(workspace.map { "Workspace \($0.name)" } ?? "No workspace")
    }

    private static let surfaceID = "shell-bar-surface"

    /// The window the column opens through.
    ///
    /// Read outwards: the rows, then a frame with BOTH dimensions fixed — the
    /// layout, settled once — then a frame with only a height, which is the
    /// part that animates, then a clip. The inner frame is what makes this
    /// safe: it answers the outer frame's shrinking proposal with the same
    /// size every time, so the rows below it never hear that anything is
    /// moving. Bottom-aligned, so the menu grows up out of the bar and closes
    /// back down into it — and so that a window shorter than its content shows
    /// the BOTTOM of it, which is the row nearest the bar and, since the list
    /// reads top to bottom, the LAST tab.
    @ViewBuilder
    private var columnWindow: some View {
        if showsColumn, fullColumnHeight > 0 {
            ShellColumn(
                tabs: tabs, selection: columnSelection, current: currentTab,
                marks: marks, menuOpen: columnHeight > 0,
                closable: columnPinned, onClose: onClose, onTouch: onTouch,
                onChoose: onChoose)
                .frame(width: width, height: fullColumnHeight)
                // The rows move as ONE object, or they scramble.
                //
                // This is the other half of the fix and it is the half that a
                // fixed frame alone does not buy. Laying the column out once
                // stops it re-flowing, but SwiftUI still resolves every leaf's
                // position against the animating ancestor and interpolates
                // each of them SEPARATELY — so a window whose height springs
                // open drags every row's text and every row's mark along its
                // own path, and three rows that are 44 points apart at rest
                // arrive from 10 points apart with the words piled on each
                // other. Watched frame by frame it looks exactly like a
                // re-flow and it is not one: the layout was right the whole
                // time and only the animation was wrong.
                //
                // `geometryGroup` resolves this subtree's geometry before its
                // children see it, so they move rigidly with it. It is the
                // modifier that exists for precisely this, and it belongs on
                // the fixed frame rather than further out — the group has to
                // be the thing whose layout is settled.
                //
                // **Measured inert now that the column is a `List`, and kept
                // anyway.** Traced frame by frame with the menu spring slowed
                // to an 8-second response, removing this line produced a
                // byte-identical set of row positions: a list's rows are laid
                // out by UIKit inside one hosting layer, so they already travel
                // rigidly and there is nothing left for the group to gather.
                // The line stays because it costs a modifier and it is the
                // documented protection for the thing above it, and because
                // what makes it inert is an implementation detail of `List`
                // rather than a promise anybody made. The frame BELOW it is not
                // inert — removing that one still scrambles the open, and the
                // trace is in the same report.
                .geometryGroup()
                .frame(height: max(0, min(columnHeight, fullColumnHeight)), alignment: .bottom)
                .clipped()
                // The rows go with the surface, and on the SAME animation as
                // everything else here — no `.animation` modifier of this
                // view's own, anywhere in this file.
                //
                // That is a rule learned the hard way. A scoped
                // `.animation(_:value:)` around the window's frame animates
                // that frame without animating the stack, the glass shape or
                // the surface's own height, which are resolved by the caller's
                // transaction instead: the glass sprang open at one speed
                // while the rows inside it were laid out at another, and what
                // was on screen for a third of a second was a sliver of one
                // row hanging at the top of an empty menu. Moved outwards onto
                // the stack it got worse, not better — the glass took the
                // animated size and the CONTENT took the target one, so the
                // bar row itself drew at the top of a surface twice its
                // height.
                //
                // One transaction has to own the whole surface, and the only
                // place that can be written is the place that changes the
                // state: `ShellRootView.syncMenu`, which opens and shuts this
                // inside a `withAnimation(ShellMotion.menu)` of its own.
                .opacity(columnHeight > 0 ? 1 : 0)
                // And out of the accessibility tree entirely while it is shut.
                //
                // There are three bars on the track and each one builds its
                // whole column, so nine rows exist for the three that are on
                // screen — the two neighbours' six, a page off the display, and
                // this bar's own three behind a closed window. None of the nine
                // is reachable by a finger, and a furled menu that still read
                // out its rows would be VoiceOver describing a menu that is not
                // open.
                //
                // It has to be HERE and not on the rows. `neighbourBar` already
                // says `accessibilityHidden(true)` and `ShellBar` already
                // ignores its own children, and a `List` leaks through both:
                // its rows are hosted as elements of their own. Measured before
                // this line — three matches for every row identifier, and
                // XCUITest refusing to press any of them.
                //
                // `columnHeight` and not `menuOpen`, so the rows stay in the
                // tree for the whole of the close: the height is a step the
                // caller's transaction interpolates, so this goes false when
                // the menu has finished shutting rather than when it started.
                .accessibilityHidden(columnHeight <= 0)
        }
    }

    /// The bar's own row: ribbon, then name, then the server.
    ///
    /// That order, and not an arbitrary one. The ribbon is the part read at a
    /// glance and the part that is the same shape every time, so it anchors
    /// the left edge where the eye already is; the name is what varies and
    /// gets the room to truncate. A ribbon pushed to the right edge moves with
    /// the length of the name, which is the one thing a mark you are meant to
    /// learn must never do.
    @ViewBuilder
    private var barRow: some View {
        Group {
            if let workspace {
                HStack(spacing: 12) {
                    ShellRibbon(
                        tabs: workspace.tabs, current: currentTab, marks: marks,
                        menuOpen: columnHeight > 0,
                        // While the menu is open the dot's frame comes from
                        // the row it is standing in, so the drawing has to
                        // agree with that row or a dot would be elongated to
                        // one width and drawn at another.
                        menuMark: columnHeight > 0 ? columnSelection : nil)
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
        .frame(width: width, height: ShellMetrics.barRow)
    }
}
