import SwiftUI

// The navigation shell: three panes on a track, the bar that is the workspace,
// and the overview at the end of the same drag.
//
// This view owns the state and reads the finger. Every threshold it applies
// comes out of `AgentKit/ShellNavigation.swift`, which is where they can be
// tested without a simulator; nothing here decides anything a `swift test`
// could have checked. What is here and could not be there is the part that is
// about SwiftUI itself: the retained track, the axis lock's lifetime, and the
// no-bounce commit.
//
// ## The invariant, and the new way there now is to break it
//
// **A pane must never be rebuilt, and its host's identity must never change.**
// Three comments in this codebase exist because that was got wrong three
// separate ways — `WorkspaceView.swift:83-97` (a pane mounted and never
// destroyed, because tearing one down costs a scroll position, a half-typed
// message and a tmux renegotiation), `FleetView.swift:1183-1202` (a route
// whose lookup drove the view structure threw every pane away the moment the
// lookup stopped succeeding), `Connection.swift:190-199` (writing focus back
// into the path changed a path element's value, and SwiftUI is free to rebuild
// a destination whose value changed).
//
// A swipeable track adds a fourth way that does not exist anywhere in the app
// today: a lazy paging container is free to RECYCLE. `TabView(.page)` and
// `LazyHStack` both are, and either would hand pane B the view pane A was
// using — the same loss, arrived at from a direction none of those three
// comments is watching. So the track is a plain `HStack` of exactly three
// slots, each keyed `.id(tab.id)`, and it stays that way when the placeholders
// below become real terminals.
//
// ## What is a placeholder here and what is not
//
// The PANES are placeholders — this commit is the shell behind a DEBUG flag,
// driven over canned fleets, and wiring it to a real fleet is a later commit.
// The container is not. It is built the way it has to be built for the commit
// that puts terminals in it, because a track that recycles is not a thing that
// can be discovered by looking at text placeholders: it looks perfect right up
// until the day it costs somebody a message they were typing.

/// The shell, over a fleet, with panes the caller supplies.
///
/// Generic over the pane so this file never has to know what a pane is. The
/// harness hands it text; the commit that wires this to a runner hands it the
/// real thing, and the retained-set rules that go with a real pane —
/// `isVisible` staying exactly one pane, the single writer for
/// `Notifier.shared.visibleTerminal` — belong to whatever it hands in, not
/// here. `DockedBar.swift:34-41` is why that matters: an input accessory lives
/// in the KEYBOARD's window, so two panes that both think they are visible are
/// two composers fighting over first responder, and mid-gesture there are
/// always two panes partly on screen.
struct ShellRootView<Pane: View>: View {
    let fleet: ShellFleet
    private let pane: (ShellPosition, ShellWorkspace, ShellTab, Bool) -> Pane

    /// Where the shell is. The one thing a commit re-seats.
    @State private var position: ShellPosition

    // MARK: The drag channel
    //
    // Four properties, and the important thing about them is what is NOT here:
    // `columnPinned` lives below, outside this group, and is never derived
    // from `lift`. The prototype keeps `colOpen` distinct from `dragY`
    // deliberately, and the bug that taught it — a tap that toggled the wrong
    // way — is what happens when one of them is computed from the other. One
    // source of truth per thing.

    /// How far the bar has been lifted, up-positive, floored at zero.
    @State private var lift: CGFloat = 0
    /// How far the track has been dragged sideways, in points.
    @State private var trackX: CGFloat = 0
    /// The axis, decided once on the first 6 points and never revisited.
    @State private var axis: ShellAxis?
    /// Which sequence this gesture walks. Set when the finger goes down, not
    /// when the axis is decided: the three panes on the track depend on it,
    /// and they are drawn from the first frame of the gesture.
    @State private var track: ShellTrack = .content
    /// Whether a gesture is in flight, so the first `onChanged` can be told
    /// from every later one. A `DragGesture` has no `onBegan`.
    @State private var gestureActive = false
    /// Whether the column was already pinned open when this gesture started.
    /// Only the tap branch reads it, and it is the reason `.toggleColumn` can
    /// be a decision rather than a guess.
    @State private var wasOpen = false

    /// The column, held open by a tap. Separate from `lift`, see above.
    @State private var columnPinned = false
    /// Whether the overview is the thing on screen.
    ///
    /// Seeded rather than always false, so the harness can open ON it. A state
    /// only reachable by performing a gesture is a state nobody screenshots,
    /// and the overview is the surface most worth looking at repeatedly — it
    /// is where forty workspaces have to stay scannable.
    @State private var overview: Bool
    @State private var overviewSearch = ""

    init(
        fleet: ShellFleet,
        initial: ShellPosition,
        openingOnOverview: Bool = false,
        @ViewBuilder pane: @escaping (ShellPosition, ShellWorkspace, ShellTab, Bool) -> Pane
    ) {
        self.fleet = fleet
        self.pane = pane
        _position = State(initialValue: initial)
        _overview = State(initialValue: openingOnOverview)
    }

    /// While tracking: the pane follows the finger, with just enough spring
    /// that a fast flick does not look linear.
    private static var tracking: Animation { .interactiveSpring }

    /// On release. Every use of it is interruptible — nothing below gates
    /// input on an animation being finished, so a second swipe onto a
    /// settling one inherits its velocity rather than waiting for it.
    private static var settle: Animation { .spring(response: 0.3, dampingFraction: 0.82) }

    /// The page's real width, read rather than stored.
    ///
    /// This was `@State` fed by `.onGeometryChange`, and that is a trap worth
    /// leaving a sign on: the width being written is the width of a stack
    /// whose own children are sized FROM it — the track is three pages wide —
    /// so the write happens inside the layout pass that the write invalidates.
    /// It does not crash and it does not warn. The hosting view simply never
    /// resolves and the whole shell renders as a blank white screen, with the
    /// only clue a trap deep inside a glass geometry effect. A width that is
    /// read on the way down cannot feed back into the pass that produced it.
    var body: some View {
        GeometryReader { geo in shell(page: geo.size.width) }
    }

    private func shell(page: CGFloat) -> some View {
        ZStack(alignment: .bottom) {
            paneTrack(page: page)
                // The page recedes as the overview arrives. Scale AND opacity,
                // from the mechanics doc: either alone reads as a transition
                // to somewhere, and both together read as this screen going
                // back to make room.
                .scaleEffect(1 - overviewProgress * 0.06)
                .opacity(1 - overviewProgress * 0.8)
                .allowsHitTesting(!overview)

            barLayer(page: page)
                .opacity(1 - overviewProgress * 0.9)
                .allowsHitTesting(!overview)

            if overviewProgress > 0 {
                ShellOverview(
                    fleet: fleet, current: position.workspace, search: $overviewSearch,
                    onOpen: open(workspace:), onDismiss: closeOverview)
                    .opacity(overviewProgress)
                    .scaleEffect(0.92 + overviewProgress * 0.08)
                    .allowsHitTesting(overview)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .overlay(alignment: .topLeading) { probe }
    }

    // MARK: - The track

    /// Three panes side by side, translated. Never lazy, never a `TabView`.
    ///
    /// Which three depends on the track the gesture started on: from the bar
    /// they are the adjacent WORKSPACES at their first tabs, and from the
    /// content they are the adjacent TABS along the flat sequence. Both
    /// neighbours are genuinely mounted and drawn at 0.72, which is what makes
    /// the incoming pane real rather than something that appears on commit.
    private func paneTrack(page: CGFloat) -> some View {
        HStack(spacing: 0) {
            slot(previousStep?.position, page: page).opacity(0.72)
            slot(position, page: page)
            slot(nextStep?.position, page: page).opacity(0.72)
        }
        // Three page-wide slots centred in one page puts the middle one on
        // screen and the neighbours exactly one page off each edge, which is
        // the prototype's `-PAGE_W + dx` written as a layout rather than as an
        // offset that has to be kept in step with the width.
        .frame(width: page * 3)
        .offset(x: trackX)
        // A FIXED width, and this is not interchangeable with the
        // `.frame(maxWidth: .infinity)` that was here first.
        //
        // A flexible frame's lower bound is its CHILD's width when no
        // `minWidth` is given: `maxWidth: .infinity` over a three-page track
        // reports three pages, not one. Nothing warns. The shell simply
        // becomes 1206 points wide inside a 402-point screen, the bar centres
        // itself 600 points off the right edge, and the screen renders empty —
        // and if anything then MEASURES that width and feeds it back in, the
        // track grows again every pass until the hosting view stops resolving
        // and the whole app goes white. Both of those happened here in that
        // order. A fixed frame is what a page is.
        .frame(width: page)
        .frame(maxHeight: .infinity)
        .clipped()
        .contentShape(.rect)
        .gesture(contentGesture(page: page))
    }

    @ViewBuilder
    private func slot(_ at: ShellPosition?, page: CGFloat) -> some View {
        Group {
            if let at, fleet.contains(at) {
                let workspace = fleet.workspaces[at.workspace]
                let tab = workspace.tabs[at.tab]
                // Keyed on the TAB's id and nothing else. Not on the index,
                // not on the position: an id that moves when the fleet
                // reorders is an id that rebuilds a pane the reorder did not
                // touch, which is `Connection.swift:190-199`'s bug wearing a
                // different hat.
                pane(at, workspace, tab, at.workspace != position.workspace)
                    .id(tab.id)
            } else {
                // The end of the fleet. Empty rather than absent, so the
                // track keeps its three slots and the middle one keeps its
                // position — a two-slot `HStack` would shift the pane you are
                // looking at sideways the moment you reached an end.
                Color.clear
            }
        }
        .frame(width: page)
    }

    private var previousStep: ShellStep? {
        fleet.step(from: position, .previous, along: track)
    }

    private var nextStep: ShellStep? {
        fleet.step(from: position, .next, along: track)
    }

    // MARK: - The bar

    private func barLayer(page: CGFloat) -> some View {
        ShellBar(
            fleet: fleet,
            position: position,
            previous: previousStep?.position.workspace,
            next: nextStep?.position.workspace,
            railWidth: ShellMetrics.railWidth(page: page),
            railX: railX(page: page),
            columnHeight: ShellGesture.columnHeight(
                up: lift, tabCount: tabCount, pinned: columnPinned),
            columnSelection: columnSelection,
            // The column dissolves as the overview arrives — all the way,
            // where the bar only fades to a tenth. The two are one surface, so
            // the column has to be gone before the overview is, or a piece of
            // glass with rows in it sits behind the grid.
            columnOpacity: 1 - overviewProgress)
            .padding(.bottom, 12)
            .gesture(barGesture(page: page))
    }

    /// How far the bar's rail has moved.
    ///
    /// **Zero unless the swipe will actually change workspace.** Within a
    /// workspace the bar holds still, because the workspace is not changing —
    /// a bar that slid for every tab swipe would be saying something false
    /// twice a swipe. When it does move it moves PROPORTIONALLY: one rail
    /// width to the content's one page, so the two arrive together.
    private func railX(page: CGFloat) -> CGFloat {
        guard let direction = ShellGesture.direction(dx: trackX),
            let step = fleet.step(from: position, direction, along: track),
            step.crossesWorkspace
        else { return 0 }
        return trackX * (ShellMetrics.railWidth(page: page) / page)
    }

    private var tabCount: Int { fleet.tabCount(ofWorkspace: position.workspace) }

    /// Which column row is under the finger, or which one you are on when the
    /// column is held open.
    ///
    /// A pinned column highlights the CURRENT tab, which is the only thing it
    /// could be highlighting: nothing is being dragged, so there is no finger
    /// to follow, and a pinned column with nothing highlighted would be a list
    /// that had forgotten where you are.
    private var columnSelection: Int? {
        if columnPinned && lift == 0 { return position.tab }
        return ShellGesture.columnSelection(up: lift, tabCount: tabCount)
    }

    private var overviewProgress: CGFloat {
        overview ? 1 : ShellGesture.overviewProgress(up: lift, tabCount: tabCount)
    }

    // MARK: - The gestures

    /// `minimumDistance: 0` so a TAP arrives here too.
    ///
    /// The tap is not a separate `TapGesture`: it is this gesture ending
    /// without ever having decided an axis, which is what the mechanics doc
    /// means by "no axis at all — that is a tap". Two recognizers would be two
    /// things racing over the same touch, and the loser would be whichever one
    /// SwiftUI felt like.
    ///
    /// `.global`, and on this gesture it is load-bearing rather than tidy. A
    /// drag's translation is the difference between two points measured in the
    /// chosen space, and the LOCAL space of this view moves while the gesture
    /// runs: the column unfurls upward, so the bar's own origin rises by
    /// exactly the lift being measured. Measured locally, `up` came out as
    /// `drag - lift`, which settles at half the distance the finger actually
    /// travelled — a column that stops at 30 points for a 60-point drag and an
    /// overview that needs twice its documented reach. Nothing about it looks
    /// like a bug; it just feels heavy.
    private func barGesture(page: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .global)
            .onChanged { value in
                begin(on: .bar)
                let dx = value.translation.width
                let up = -value.translation.height
                decideAxis(dx: dx, up: up)
                withAnimation(Self.tracking) {
                    switch axis {
                    case .horizontal:
                        trackX = ShellGesture.translation(dx: dx, rubberBanding: rubberBands(dx))
                    case .vertical:
                        lift = max(0, up)
                    case nil:
                        break
                    }
                }
            }
            .onEnded { value in
                let decided = axis
                let dx = value.translation.width
                let up = lift
                let openedBefore = wasOpen
                rest()
                apply(
                    fleet.barRelease(axis: decided, dx: dx, up: up, at: position), dx: dx,
                    page: page, wasOpen: openedBefore)
            }
    }

    /// The content's own swipe, along the flat sequence.
    ///
    /// Also `minimumDistance: 0`, and that is safe here only because the panes
    /// in this commit are placeholders that want no touches. The commit that
    /// puts a terminal in the slot has to give this a minimum distance or hand
    /// the pane the touch first — a terminal's own pan is its scrollback, and
    /// `TerminalScrollTests` is the regression that says so.
    private func contentGesture(page: CGFloat) -> some Gesture {
        // `.global` for the same reason the bar's is, kept the same here so
        // the two gestures cannot come to measure different things.
        DragGesture(minimumDistance: 0, coordinateSpace: .global)
            .onChanged { value in
                begin(on: .content)
                let dx = value.translation.width
                decideAxis(dx: dx, up: -value.translation.height)
                guard axis == .horizontal else { return }
                withAnimation(Self.tracking) {
                    trackX = ShellGesture.translation(dx: dx, rubberBanding: rubberBands(dx))
                }
            }
            .onEnded { value in
                let decided = axis
                let dx = value.translation.width
                rest()
                apply(
                    fleet.contentRelease(axis: decided, dx: dx, at: position), dx: dx,
                    page: page, wasOpen: false)
            }
    }

    /// The first `onChanged` of a gesture, and only the first.
    private func begin(on which: ShellTrack) {
        guard !gestureActive else { return }
        gestureActive = true
        track = which
        wasOpen = columnPinned
    }

    /// Decide the axis, once.
    ///
    /// The guard is the whole rule: once `axis` is non-nil nothing asks again
    /// for the rest of the gesture. An axis that could be revisited is a swipe
    /// that starts sideways and ends up opening the column, which is every
    /// accidental gesture in the design review.
    private func decideAxis(dx: CGFloat, up: CGFloat) {
        guard axis == nil else { return }
        axis = ShellGesture.axis(dx: dx, up: up)
    }

    private func rubberBands(_ dx: CGFloat) -> Bool {
        guard let direction = ShellGesture.direction(dx: dx) else { return false }
        return fleet.rubberBands(at: position, direction, along: track)
    }

    /// The drag channel goes back to rest, unconditionally, before any branch
    /// below can return.
    ///
    /// This runs FIRST in both `onEnded`s and it takes no arguments, so there
    /// is no branch it can be skipped by. That ordering is load-bearing: the
    /// column's height is read straight off `lift`, so a release that decides
    /// to do nothing and returns early would leave `lift` standing and the
    /// column open with no gesture holding it — a column that is open because
    /// of a drag that ended.
    ///
    /// `trackX` is deliberately NOT reset here. It is the track's translation
    /// rather than a fact about the gesture, every arm of `apply` resolves it
    /// exactly once, and zeroing it here would make a commit animate from the
    /// centre — which is the page jumping back before it goes.
    private func rest() {
        gestureActive = false
        axis = nil
        wasOpen = false
        withAnimation(Self.settle) { lift = 0 }
    }

    private func apply(_ release: ShellRelease, dx: CGFloat, page: CGFloat, wasOpen: Bool) {
        switch release {
        case .commit(let step):
            commit(step, dx: dx, page: page)
        case .springBack, .abandon:
            withAnimation(Self.settle) { trackX = 0 }
        case .land(let tab):
            withAnimation(Self.settle) {
                position.tab = tab
                // Landing on a row is choosing from the column, so the column
                // has done its job. It furls whether it was pinned or dragged
                // — a tap-opened column that stayed open after a choice would
                // leave the chosen pane behind a list of its siblings.
                columnPinned = false
                trackX = 0
            }
        case .openOverview:
            withAnimation(Self.settle) {
                overview = true
                columnPinned = false
                trackX = 0
            }
        case .toggleColumn:
            withAnimation(Self.settle) {
                columnPinned = !wasOpen
                trackX = 0
            }
        }
    }

    /// Animate to the neighbour, then re-seat on it without animating.
    ///
    /// The one that is easy to get wrong. Animating the track to ±one page and
    /// then setting the new position leaves `trackX` still at ±one page with
    /// the new pane already in the middle slot — so it has to go back to zero
    /// in the same breath, and if that zeroing animates you watch the page
    /// slide back to where it came from. The web prototype disables its
    /// transitions for one frame and restores them two `requestAnimationFrame`s
    /// later; SwiftUI has a first-class version and this is it.
    ///
    /// `.logicallyComplete` and not `.removed`: the spring is allowed to still
    /// be settling visually when the swap happens, which is what makes the
    /// commit feel instant rather than back-loaded. A `DispatchQueue.main.async`
    /// imitation of this would fire on a frame boundary that has nothing to do
    /// with the animation and would sometimes land early.
    private func commit(_ step: ShellStep, dx: CGFloat, page: CGFloat) {
        let sign: CGFloat = dx < 0 ? -1 : 1
        withAnimation(Self.settle, completionCriteria: .logicallyComplete) {
            trackX = sign * page
            // The column belongs to the workspace it lists, so a crossing
            // furls it. A swipe within one workspace leaves it alone: the same
            // list is still the right list.
            if step.crossesWorkspace { columnPinned = false }
        } completion: {
            var silent = Transaction()
            silent.disablesAnimations = true
            withTransaction(silent) {
                position = step.position
                trackX = 0
            }
        }
    }

    private func open(workspace index: Int) {
        withAnimation(Self.settle) {
            if fleet.workspaces.indices.contains(index) {
                position = ShellPosition(workspace: index, tab: 0)
            }
            overview = false
            overviewSearch = ""
        }
    }

    private func closeOverview() {
        withAnimation(Self.settle) {
            overview = false
            overviewSearch = ""
        }
    }

    // MARK: - The probe

    /// The one way a UI test can ask this shell where it is.
    ///
    /// The same technique `TerminalView.swift:387-392` uses and for the same
    /// reason: a gesture's outcome is a transform and a couple of indices, and
    /// there is no label on screen that says either. `TerminalScrollTests`
    /// reads `terminal-surface`'s value; `ShellGestureTests` reads this one.
    ///
    /// A one-point element in the corner rather than a value on the shell
    /// itself, because `accessibilityValue` on a container makes the container
    /// the element and hides everything inside it — which would take the bar,
    /// the cards and the panes out of the tree the same test has to swipe on.
    private var probe: some View {
        Rectangle()
            .fill(Color.white.opacity(0.001))
            .frame(width: 1, height: 1)
            .accessibilityElement()
            .accessibilityIdentifier("shell-state")
            .accessibilityValue(
                "ws=\(position.workspace) tab=\(position.tab) "
                    + "workspaces=\(fleet.workspaces.count) tabs=\(tabCount) "
                    + "column=\(Int(ShellGesture.columnHeight(up: lift, tabCount: tabCount, pinned: columnPinned).rounded())) "
                    + "pinned=\(columnPinned ? 1 : 0) overview=\(overview ? 1 : 0)")
    }
}
