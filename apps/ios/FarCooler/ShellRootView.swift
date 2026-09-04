import SwiftUI

// The navigation shell: a screen that fills the screen, the bar that is the
// workspace, and the overview at the end of the same drag.
//
// This view owns the state and reads the finger. Every threshold it applies
// comes out of `AgentKit/ShellNavigation.swift`, which is where they can be
// tested without a simulator; nothing here decides anything a `swift test`
// could have checked. What is here and could not be there is the part that is
// about SwiftUI itself: the retained track, how long an axis lasts, the
// no-bounce commit, and the transforms below.
//
// ## The invariant, and the new way there now is to break it
//
// **A pane must never be rebuilt, and its host's identity must never change.**
// Three comments in this codebase exist because that was got wrong three
// separate ways — the pane host mounted a pane and never destroyed it, because
// tearing one down costs a scroll position, a half-typed message and a tmux
// renegotiation; a route whose lookup drove the view structure threw every
// pane away the moment the lookup stopped succeeding; and
// `Connection.swift:178-206`, where writing focus back into the navigation
// path changed a path element's value and SwiftUI is free to rebuild a
// destination whose value changed. The first two screens are gone and the
// rules they were written on are in `ShellPaneTrack.swift:5-30`.
//
// A swipeable track adds a fourth way that does not exist anywhere in the app
// today: a lazy paging container is free to RECYCLE. `TabView(.page)` and
// `LazyHStack` both are, and either would hand pane B the view pane A was
// using — the same loss, arrived at from a direction none of those three
// comments is watching. So the track is a plain `HStack` of exactly three
// slots, each keyed `.id(tab.id)`, and it stays that way when the placeholders
// below become real terminals.
//
// It is also why the two horizontal weights below are ONE pane track and a
// separate bar track rather than three whole screens side by side. A screen
// that carried its own pane would put the same tab in the tree twice — once in
// the heavy track and once in the light one — and two views claiming one id is
// the recycling problem wearing its last remaining hat.
//
// ## What is a placeholder here and what is not
//
// The PANES are placeholders — this commit is the shell behind a DEBUG flag,
// driven over canned fleets, and wiring it to a real fleet is a later commit.
// The container is not. It is built the way it has to be built for the commit
// that puts terminals in it, because a track that recycles is not a thing that
// can be discovered by looking at text placeholders: it looks perfect right up
// until the day it costs somebody a message they were typing.
//
// ## Where the rest of it went
//
// This file is the CONTAINER: the state, the layering, and the two tracks. The
// other two thirds of what it used to be are one seam apart each and are
// separate files now.
//
// - `ShellPageLayer.swift` — the page as one moving object: the shape it is
//   clipped to, the card it carries, and the transforms. Its arithmetic is
//   `AgentKit/ShellFlight.swift`, which `swift test` can reach.
// - `ShellDrag.swift` — the finger: the axis and the handovers between its
//   two directions, the drag channel, and the seven things a release can be.
//   Its decisions are `ShellGesture`, `ShellBarDrag` and
//   `ShellFleet.barRelease`, all of which `swift test` can also reach.
//
// Three files, one type, and that is why the state below is not `private`: an
// extension in another file cannot see a private member. It is still not
// reachable from a CALLER — `init` takes an initial position and a pane
// builder, and hands back nothing — which is the property `onRest` exists to
// keep and the one the comment there is about.

/// The shell, over a fleet, with panes the caller supplies.
///
/// Generic over the pane so this file never has to know what a pane is. The
/// harness hands it text; `ShellScreen` hands it a terminal, an agent or a
/// diff.
///
/// The retained set is NOT here, and that is the one structural thing to know
/// about this file. `ShellPaneTrack` owns which panes are mounted, how long
/// they stay, and which single one is `isVisible` — read its header before
/// changing anything below that moves `position`. `DockedBar.swift:34-41` is
/// why it matters: an input accessory lives in the KEYBOARD's window, so two
/// panes that both think they are visible are two composers fighting over
/// first responder, and mid-gesture there are always two panes partly on
/// screen.
struct ShellRootView<Pane: View, Actions: View>: View {
    let fleet: ShellFleet
    /// What to call the runner `fleet` is on, and the worktrees on the others.
    ///
    /// Both reach exactly one view — `ShellOverview` — and neither is part of
    /// the fleet. That is not an accident of plumbing, it is the rule:
    /// `ShellPosition` indexes into `fleet.workspaces`, the bar walks it and
    /// `ShellPaneTrack` mounts a pane for every tab it steps onto, so a
    /// workspace with no connection behind it must never be in that array.
    /// See `ShellServerGroup`, which says so at length.
    private let liveServer: String?
    private let elsewhere: [ShellServerGroup]
    /// A card on another runner, tapped.
    private let onCross: (ShellServerGroup, ShellWorkspace) -> Void
    private let pane: (ShellPaneSlot) -> Pane
    /// What the overview puts in its navigation bar. See
    /// `ShellOverview.actions`.
    private let overviewActions: () -> Actions
    /// A tab to go to, by id, honored once and cleared.
    ///
    /// **The one way in from outside, and deliberately not a binding to
    /// `position`.** That value is re-seated inside a silent transaction at the
    /// end of every commit, and a caller holding a binding to it could write
    /// into the frame a spring is settling — the exact shape
    /// `Connection.swift:190-199` records as having thrown every pane away
    /// once. A REQUEST is a different thing: it is honored on this view's own
    /// terms, in one place, and it is cleared as it is taken so a second card
    /// naming the same terminal reads as a new request rather than as a value
    /// that has not changed.
    ///
    /// One-shot for the reason the pane host's `requested` was one-shot, which is
    /// the same mechanism this replaces: the shell moves on afterwards without
    /// telling anybody, so a request that stayed set would name a pane nobody
    /// is on and block the next tap on the same card.
    ///
    /// A tab id and not a terminal id, because a tab is what the shell has
    /// positions for — `ShellFleet.position(ofTab:)` is the whole of the
    /// lookup, and the Changes tab has no terminal to name. Resolving the one
    /// into the other is `ShellScreen.requestedTab`, which is where the fleet
    /// and the shell's vocabulary are both in hand.
    ///
    /// Not `private` for the same reason `position` is not: `honorRequest` is
    /// written in `ShellDrag.swift`, which is a fact about the three files this
    /// one type is spelled across rather than about what a caller can hold. The
    /// only way in from outside is still `init`.
    @Binding var request: String?
    /// Called when the pane AT REST changes, and at no other time.
    ///
    /// The shell's `position` is not reachable from outside and has to stay
    /// that way — it is re-seated inside a deliberately silent transaction, and
    /// a binding out of it would let a caller write back into the one value a
    /// commit is in the middle of settling. (The property lost the `private`
    /// keyword when the gesture moved to `ShellDrag.swift`, which is a fact
    /// about the three files this one type is written in and not about what a
    /// caller can hold: `init` takes a fleet, a position and a pane builder,
    /// and nothing hands the state back out.) What a caller genuinely needs is the EVENT, and only
    /// the one: `ShellScreen` is the single writer of
    /// `Notifier.shared.visibleTerminal`, which `Notifications.swift:162` reads
    /// to suppress a banner about the pane you are looking at and
    /// `Connection.markVisibleSeen()` reads to claim the runner's ten-second
    /// watch.
    ///
    /// Never fired mid-gesture. Two panes are on screen for the whole of a
    /// swipe and neither of them has arrived; `position` moves once, when the
    /// release lands.
    private let onRest: ((ShellPosition) -> Void)?

    /// Where the shell is. The one thing a commit re-seats.
    @State var position: ShellPosition

    // MARK: The drag channel
    //
    // The properties a gesture writes, and the important thing about them is
    // what is NOT here:
    // `columnPinned` lives below, outside this group, and is never derived
    // from `lift`. The prototype keeps `colOpen` distinct from `dragY`
    // deliberately, and the bug that taught it — a tap that toggled the wrong
    // way — is what happens when one of them is computed from the other. One
    // source of truth per thing.

    /// How far the bar has been lifted, up-positive, floored at zero.
    @State var lift: CGFloat = 0
    /// How far the finger is above the bar's own top edge — a PLACE, not a
    /// travel, and the number the PAGE answers to.
    ///
    /// `lift` and this one are the same number until a redirect charges one
    /// of them, or until a gesture starts somewhere other than the bar's own
    /// top edge, and after that only this one is a fact about where the
    /// finger actually is. See `ShellBarDrag.startAbove` and `Frame.above`,
    /// which is what this is written from, on every frame the vertical owns
    /// the gesture; `ShellPageLayer.pageRise` and `menuShouldShow` are its
    /// two readers, replacing what `lift` and `fingerLift` used to answer
    /// for them respectively.
    @State var pageAbove: CGFloat = 0
    /// How far the track has been dragged sideways, in points.
    @State var trackX: CGFloat = 0
    /// How far the finger has carried a LIFTED page sideways, in points.
    ///
    /// A second horizontal channel, and not the same one as `trackX`, because
    /// the two move different things by different amounts. `trackX` is the
    /// three-pane track's own translation, in PAGE coordinates behind a clip
    /// that is a page wide; `carryX` moves the whole shrunken card, on screen,
    /// one point per point of finger. Feeding a held card from `trackX` would
    /// scale the finger by the card's own shrink — a 70-point thumb move
    /// sliding the card 29 points — which is the opposite of following it.
    ///
    /// Zero for the whole column phase. Until the page has left the display
    /// there is nothing in your hand to move sideways, and `dx` is the arc a
    /// thumb draws travelling up a phone; see `ShellGesture.pageIsHeld`.
    @State var carryX: CGFloat = 0
    /// How far into the overview the shell is, 0…1.
    ///
    /// **Stored, not derived from `lift`**, and that is a bug fix rather than
    /// a preference. It used to be `overview ? 1 : progress(lift)`, and `lift`
    /// is the one thing `rest()` zeroes unconditionally the instant a finger
    /// leaves. On the release that opens the overview there is a render
    /// between `rest()` zeroing the lift and `flyToCell` setting `overview` —
    /// and in that render this read 0, which took the grid's mount condition
    /// with it. The overview was torn out of the tree and re-inserted a frame
    /// later, so what the owner saw at the end of every lift was the WHOLE
    /// GRID fading back in from black while the page flew, with the bar
    /// flashing back to full strength on the way. It looks exactly like the
    /// card fading in at the end of the flight, and it is not: it is the
    /// screen behind it.
    ///
    /// The same argument `crossing` makes below. This is the shell's SHAPE,
    /// which each release resolves exactly once, rather than a fact about the
    /// gesture, which `rest()` clears.
    @State var reveal: CGFloat = 0
    /// The page's own alpha over the grid.
    ///
    /// Only ever 1, 0, or on its way between them over `handover`. What it is
    /// NOT is a crossfade partner: at every moment of a handover the OTHER
    /// half — the card in the cell — is fully drawn underneath, so this fades
    /// over something opaque and there is no frame where neither is all
    /// there. See `land()`.
    @State var pageAlpha: CGFloat
    /// Whether the grid holds the current workspace's cell open rather than
    /// drawing a card in it.
    ///
    /// Separate from `flights` on purpose, and the separation is the whole of
    /// the fix for the landing. The cell has to be a hole for as long as the
    /// page is in the AIR — a workspace cannot be in two places — but it has
    /// to stop being one the moment the page arrives, while the page is still
    /// opaque on top of it and a frame before the page begins to dissolve.
    /// One flag for both meant the card could only appear by fading in as the
    /// page faded out, and two half-present layers over one rectangle is a
    /// dip: for an instant you see through both of them to the ground, which
    /// is the "vanishes into the hole" the owner called out.
    @State var cellIsHole: Bool
    /// How much of a card the pages have become for a sideways crossing, 0…1.
    ///
    /// Stored rather than derived from `trackX`, and this is the one property
    /// here that could look redundant. `trackX` is re-seated SILENTLY at the
    /// end of a commit — the new pane is already in the middle slot, so
    /// zeroing the translation is invisible — but the card is not: it is at
    /// 96% with the display's corners, and snapping it back to a full-bleed
    /// page in the same silent frame is a pop exactly where the gesture is
    /// supposed to finish. So the crossing unwinds on its own spring, after
    /// the re-seat, and the page grows back into the screen.
    @State var crossing: CGFloat = 0
    /// How much of a CARD the page has become, 0…1 — its shape, its size and
    /// its face, all on one number.
    ///
    /// **The animatable value the flight runs on, and it exists because a
    /// boolean cannot be interpolated.** The crop used to read `isFlying ?
    /// card : pageFrame.height`, which is a step, and `flights` is incremented
    /// OUTSIDE the animation that carries the page — it has to be, because the
    /// grid's mount and the landing's overtaken-guard both read it. So the
    /// page's height changed in an update with no animation in it and then the
    /// spring translated what was already a card: at release the page abruptly
    /// halved in height and only then flew, and tapping a card ran the same
    /// bug backwards — the page grew into the bottom 36% of the screen and the
    /// top of it appeared in one frame when the spring finished.
    ///
    /// Every part of "is it a card yet" now hangs off this one number, so
    /// there is nothing left that can be a step: the scale between
    /// `ShellMotion.heldScale` and the cell's own, the crop between a whole
    /// page and a card's rectangle, the corner between the display's and the
    /// card's, and the alpha of the card FACE the page carries. Set only
    /// inside the animation that moves the page, in both directions.
    ///
    /// Zero for the whole of a lift, and that is the property the owner asked
    /// for rather than a consequence: a page being minimized is still a page
    /// the whole way up. Becoming a card happens after you let go, because
    /// becoming a card is the same event as landing in the grid.
    @State var cropped: CGFloat
    /// Where the finger that is lifting the page went down, in the page's own
    /// coordinates, or nil before anything has been lifted.
    ///
    /// The point the shrink is anchored at. The page used to shrink about the
    /// middle of the display, which is fine only if that is where your thumb
    /// is: anywhere else, the pixels under the finger slide toward the centre
    /// as the page gets smaller, so a card that is not being moved sideways at
    /// all appears to slide sideways out from under the thumb holding it.
    /// Anchoring at the touch-down point makes the one point of the page you
    /// are actually holding the one point that does not move.
    ///
    /// The START of the gesture and not the finger's current position. A live
    /// anchor would re-aim the shrink every frame, which is a second sideways
    /// motion on top of `carryX` and the same class of mistake as letting the
    /// destination cell pull the page mid-drag.
    @State var liftOrigin: CGFloat?
    /// The CONTENT gesture's axis, decided once on the first 6 points and
    /// never revisited.
    ///
    /// **The bar's is not here**, and the split is the point rather than an
    /// oversight: the two surfaces differ in how long an axis lasts, so they
    /// differ in what holds it. Vertical on the content is the terminal's
    /// scrollback and there is a second party to yield it to, so the answer
    /// is given once; on the bar nothing else is listening, so it is re-asked
    /// every frame and lives in `barDrag`. `ShellGesture.lean` is where that
    /// difference is argued.
    @State var axis: ShellAxis?
    /// Which sequence this gesture walks. Set when the finger goes down, not
    /// when the axis is decided: the three panes on the track depend on it,
    /// and they are drawn from the first frame of the gesture.
    @State var track: ShellTrack = .content
    /// Whether a gesture is in flight, so the first `onChanged` can be told
    /// from every later one. A `DragGesture` has no `onBegan`.
    @State var gestureActive = false
    /// Whether the column was already pinned open when this gesture started.
    /// Only the tap branch reads it, and it is the reason `.toggleColumn` can
    /// be a decision rather than a guess.
    @State var wasOpen = false

    /// The column, held open by a tap. Separate from `lift`, see above.
    @State var columnPinned = false

    /// How far a finger has taken the page back out of the grid, 0…1.
    ///
    /// **The overview's dismissal, as a tracked value rather than as a
    /// threshold.** It is the reverse of `reveal`, and it exists for the
    /// reason `reveal` does: the page's place, size and shape are all
    /// continuous functions of a gesture, and the way OUT of this screen used
    /// to be the one part of the journey that was not — a `DragGesture` whose
    /// `onChanged` recorded a boolean and whose `onEnded` either dismissed or
    /// did not. WWDC 2018 803, on exactly that: *"avoid methods that are only
    /// detected at the end of the gesture."*
    ///
    /// Read by `ShellPageLayer.flightOffset` through `ShellFlight.returning`,
    /// which is where the arithmetic is and where `swift test` can reach it.
    /// Zero is the cell and one is the display, and at zero the offset it
    /// produces is byte for byte the landed one — so this being here costs a
    /// page that is not being pulled exactly nothing.
    @State var pullOut: CGFloat = 0

    /// Whether a finger is on that pull right now.
    ///
    /// Separate from `pullOut > 0`, and for the same reason `columnPinned` is
    /// separate from `lift`: the first frame of a pull is at zero progress and
    /// still has to do the handover, and the last frame of an abandoned one is
    /// back at zero while a spring is still finishing. One source of truth per
    /// thing — a finger owns this, a spring owns `pullOut`.
    @State var pullingOut = false

    /// How far the pull had come, and when, the last time it actually moved.
    ///
    /// The overview's own copy of `lastMoved`, and it is here for exactly the
    /// reason that one is: `DragGesture.Value.velocity` does not fall to zero
    /// when a finger stops. A pull dragged half way and PARKED still reports
    /// enough speed to project past the threshold, so without this the one
    /// gesture the tracking exists to make abandonable — go half way, think
    /// better of it, hold still, let go — would dismiss anyway. Same 60
    /// milliseconds, same half point of slop, same argument.
    ///
    /// The clock is read in the handler rather than off `DragGesture.Value`,
    /// which would mean a third argument through two closures for a difference
    /// of one dispatch: the handler runs synchronously from the gesture
    /// callback, and the threshold it feeds is sixty milliseconds wide.
    @State var pullMoved: (down: CGFloat, at: Date)?

    /// How far the finger is above the bar row's top edge, right now, or nil
    /// when no finger is on an open column.
    ///
    /// **The highlight follows the finger, not the travel.** The column's
    /// selection used to be `ShellGesture.columnSelection(up: lift, …)` — how
    /// far the drag had COME, mapped a row per 44 points — and that is a
    /// different mapping from the one a tap goes through, which measures from
    /// the bar's drawn bottom edge. The two agree only for a drag that began
    /// exactly on the bar row's top edge; a drag begun at its bottom lit a row
    /// one above the thumb for the whole gesture, and a 20-point lift from
    /// there lit the last row while the thumb was still 24 points below the
    /// column entirely. One mapping now, `ShellGesture.columnRow`, and this is
    /// the point it is asked about.
    ///
    /// A separate `@State` from `lift` rather than something derived from it,
    /// for the reason `lift` cannot answer it at all: a lift is a distance and
    /// a row is a place, and the difference between them is where the finger
    /// went down — which is a fact about the gesture and not about the shell.
    /// `ShellDrag` writes it on every frame of a vertical drag and clears it
    /// in `rest()`.
    @State var fingerAbove: CGFloat?

    /// Where the finger was, and when, the last time it actually moved.
    ///
    /// **`DragGesture.Value.velocity` does not fall to zero when a finger
    /// stops, and this is what stands in for the zero it should have
    /// reported.** Measured on a simulator through the shell's own probe: a
    /// 60-point drag released after holding still for 300ms still reports 131
    /// points per second, and after 500ms it still reports 47. At a scroll
    /// view's deceleration those are 65 and 23 points of projection — on a
    /// threshold of 70. So a drag that was placed deliberately, paused over
    /// its target and then released would commit a page turn that the finger
    /// never asked for, which is the exact opposite of what projecting
    /// momentum is for.
    ///
    /// The reason is the estimator rather than a bug: UIKit delivers no touch
    /// event while a finger is stationary, so there are no new samples for a
    /// velocity to decay against and the last real motion keeps most of its
    /// weight. What IS knowable is when the finger last moved, and a release
    /// more than `ShellRootView.stillFor` after that is a release from a
    /// standstill however fast the estimator still thinks it is going.
    ///
    /// Written by both gestures on every frame that moves; cleared in
    /// `begin`, so a gesture that never moved — a tap — has no movement to be
    /// recent and carries no momentum at all.
    @State var lastMoved: (at: CGSize, time: Date)?

    /// The bar surface's bottom edge, in global coordinates.
    ///
    /// What a TAP on an open column is measured against — see
    /// `ShellDrag.tappedRow`. Measured rather than recomputed from
    /// `safeArea.bottom + barGap`, which is the arithmetic that puts the bar
    /// there: a second copy of it here would be right until somebody changed a
    /// padding, and would then land every tap one row off its target with
    /// nothing on screen to say why.
    ///
    /// The BOTTOM edge and not the top, because the column grows upward out of
    /// the bar row. The top edge moves for the whole of the menu's spring and
    /// would write this on every frame of it; the bottom edge changes only
    /// when the safe area does.
    @State var barBottom: CGFloat = 0
    /// Whether the menu is showing, as a state of its own rather than as
    /// something read off the lift.
    ///
    /// **It is derived — `menuShouldShow` is the derivation — and then
    /// STORED, and the reason is the animation rather than the value.** The
    /// lift is written inside an `interactiveSpring` because a page has to
    /// follow a finger one point per point; the menu does not follow anything,
    /// it is shut and then whole, and run on the tracking spring it appeared
    /// as if it had been switched on. It needs its own transaction.
    ///
    /// A scoped `.animation(_:value:)` inside `ShellBar` was tried first and
    /// is the wrong tool twice over: it animates the modifier it is attached
    /// to while the surface around it resolves on the caller's transaction, so
    /// the glass and the rows inside it opened at two different speeds and
    /// disagreed about where they were. See the note in `ShellBar`. The state
    /// being here means one `withAnimation` owns the whole surface — the
    /// glass, the window, the rows, and the height the stack lays out against.
    @State var menuOpen = false
    /// Whether the overview is the thing on screen.
    ///
    /// Seeded rather than always false, so the harness can open ON it. A state
    /// only reachable by performing a gesture is a state nobody screenshots,
    /// and the overview is the surface most worth looking at repeatedly — it
    /// is where forty workspaces have to stay scannable.
    @State var overview: Bool
    @State var overviewSearch = ""

    /// Where every laid-out card sits, by workspace index, in screen
    /// coordinates.
    ///
    /// All of them and not just the current one, because the flight runs both
    /// ways: the page lands in the cell of the workspace you are in, and it
    /// grows back out of the cell of the workspace you TAP, which is a
    /// different cell and is not known until the tap. A grid that only
    /// published the current card would leave the second half of that
    /// journey starting from wherever the first half ended.
    @State var tiles: [Int: CGRect] = [:]
    /// The screen's own frame, so the flight has a start as well as an end.
    /// Measured through the safe area, because that is what the page fills.
    @State var pageFrame: CGRect = .zero

    /// What the pane under the finger says about the drag now under way.
    ///
    /// Owned here and handed down the environment, because the two halves live
    /// at opposite ends of the tree: a hunk's scroll view is the only thing
    /// that knows it moved, and the shell's `onChanged` is the only place the
    /// answer can be acted on. See `ShellDragClaim`.
    @State var dragClaim = ShellDragClaim()

    /// The furthest the shell's own track strayed from center during the
    /// gesture now under way, in points, and the highest it reached during the
    /// last one once that gesture is over.
    ///
    /// **A peak rather than a value, because the defect it exists to catch is
    /// a transient.** A shell that jumps sideways over a diff and then eases
    /// itself back is, at every moment a test can read `trackX`, a shell that
    /// has not moved: the drag ends with the track at zero and the tab
    /// unchanged, which is exactly what "the shell held still" looks like
    /// from outside. The owner's report is the middle of the gesture — "it
    /// keeps triggering the scroll/pan gestures for like a fraction of a
    /// second" — so the only honest measurement is the maximum, kept across
    /// the frames nobody can sample.
    ///
    /// Reset on touch-down and never during a gesture, so a test may read it
    /// after the finger has come up and still be reading that gesture.
    @State var strayed: CGFloat = 0

    /// The translation the CONTENT's axis was decided on, once per gesture.
    ///
    /// The axis is decided from one sample — the first that clears
    /// `ShellMetrics.axisLock` — and then never revisited, so which sample
    /// that is, is the whole of the decision. It is not a number anything on
    /// screen can be asked about, and a suite that could only see the outcome
    /// could not tell "the rule chose horizontal" apart from "the rule was
    /// handed forty points of sideways travel in its first frame". See
    /// `ShellGesture.contentAxis`.
    @State var lockedOn: CGSize = .zero

    /// Where in this drag the pane under the finger ran out of room.
    ///
    /// Zero for a drag no pane wanted, which is every drag over a terminal and
    /// every drag over the ground between a diff's cards. See
    /// `ShellRootView.carriedX`.
    @State var handoff: CGFloat = 0

    /// The bar gesture's axis, and what each of its two axes owes the other.
    ///
    /// **Its own type, in AgentKit, because the redirection has a lifetime
    /// and a lifetime is the one thing a pure function cannot hold.** Whether
    /// this frame is a handover depends on what the last frame answered;
    /// what a handover charges depends on where the finger was when it
    /// happened. As three pieces of `@State` and a closure that is a shape
    /// no test can reach, which is what made the axis a lock for as long as
    /// it was one. See `ShellBarDrag`.
    @State var barDrag = ShellBarDrag()

    /// How many flights between the page and its cell are in the air.
    ///
    /// A count rather than a flag so an overlap cannot strand it: releasing
    /// into the overview and tapping a card before that flight has landed
    /// starts a second one, and the first one's completion must not then
    /// announce that the page has arrived somewhere it is no longer going.
    ///
    /// What it gates is the handover. While anything is in the air the PAGE is
    /// the current workspace's card and the grid leaves that cell empty; when
    /// the air is clear the grid's own card is the card and the page is not
    /// drawn at all. Two copies of one workspace, one flying over the other,
    /// is the thing this exists to prevent.
    @State var flights = 0

    init(
        fleet: ShellFleet,
        initial: ShellPosition,
        openingOnOverview: Bool = false,
        request: Binding<String?> = .constant(nil),
        onRest: ((ShellPosition) -> Void)? = nil,
        liveServer: String? = nil,
        elsewhere: [ShellServerGroup] = [],
        onCross: @escaping (ShellServerGroup, ShellWorkspace) -> Void = { _, _ in },
        @ViewBuilder overviewActions: @escaping () -> Actions,
        @ViewBuilder pane: @escaping (ShellPaneSlot) -> Pane
    ) {
        self.fleet = fleet
        self.liveServer = liveServer
        self.elsewhere = elsewhere
        self.onCross = onCross
        self.pane = pane
        self.overviewActions = overviewActions
        _request = request
        self.onRest = onRest
        _position = State(initialValue: initial)
        _overview = State(initialValue: openingOnOverview)
        // Opening ON the overview is the same state a landing leaves behind:
        // the grid holds a real card and the page is not drawn.
        _reveal = State(initialValue: openingOnOverview ? 1 : 0)
        _pageAlpha = State(initialValue: openingOnOverview ? 0 : 1)
        _cellIsHole = State(initialValue: !openingOnOverview)
        // A shell that opens ON the overview has already landed: the page is
        // card-shaped and card-faced, waiting under a cell it is not drawn in,
        // so the first tap flies OUT correctly rather than growing a
        // full-bleed page out of a 168-point hole.
        _cropped = State(initialValue: openingOnOverview ? 1 : 0)
    }

    /// The put-back in `carriedX`, and nothing else any more.
    ///
    /// **This was once wrapped around every tracked write, and that was the
    /// lag.** `lift`, `trackX`, `carryX` and `crossing` are continuous
    /// functions of the finger, and a spring on them is a low-pass filter
    /// between the thumb and the pixels: measured at **43-45 points of offset
    /// for the whole of a drag**, with the page still travelling 24 more
    /// points 75ms after the finger stopped. `ShellFlight.offset` promises
    /// "one point of page for one point of drag" and could not keep that
    /// promise while its own input was smoothed. Putting the spring back
    /// reproduces the 43-point lag exactly, which is how we know.
    ///
    /// What survives is the correction in `carriedX` — a couple of points the
    /// shell should never have taken once a pane claimed the drag. That is not
    /// the finger's position, it is an apology for having moved, and easing an
    /// apology is right.
    static var tracking: Animation { .interactiveSpring }

    /// On release, when what moved was one pane inside a workspace.
    ///
    /// Every use of it is interruptible — nothing below gates input on an
    /// animation being finished, so a second swipe onto a settling one
    /// inherits its velocity rather than waiting for it.
    static var settle: Animation { .spring(response: 0.3, dampingFraction: 0.82) }

    /// The same release, for the directions no finger threw.
    ///
    /// **The same response and no overshoot**, so it is the same object
    /// arriving at the same speed and only the ringing is gone — the split
    /// `ShellMotion.menu` / `menuSettled` makes, made for the other spring.
    /// WWDC 2018 803 is the rule: *"start with 100% damping… if the gesture
    /// that's driving the motion itself has momentum, then you should reward
    /// that momentum with a little bit of overshoot."*
    ///
    /// Two of `settle`'s uses have no such momentum, and they are the two
    /// where the motion runs OPPOSITE to the thumb or does not exist at all.
    ///
    /// **`rest()`, and it is not cosmetic.** A lift is thrown UP and the page
    /// falls DOWN when you let go, so an overshoot there is a page rewarding a
    /// momentum it was given in the other direction. Traced frame by frame off
    /// a 200-point lift released and abandoned, `lift` ran 191 → 0 and then
    /// **through −1.38, back up through +0.0098, and sat positive for thirteen
    /// frames.** Nothing DRAWS a lift that small — every consumer clamps:
    /// `pageRise` is a `max(0,)`, `columnHeight` is a threshold,
    /// `overviewProgress` is clamped at both ends. But `ShellRootView.shell`
    /// mounts the whole overview on `lift > 0 || reveal > 0 || overview ||
    /// flying`, and THAT is not clamped. So for about a tenth of a second
    /// after a lift had visibly finished and the page was back on the display,
    /// the shell was still holding a `NavigationStack`, a `LazyVGrid` and
    /// forty cards in the tree behind a workspace nobody was looking at — the
    /// same shape of defect the menu's `columnHeight > 0` turned out to have,
    /// found the same way. At 1.0 the trace is monotone and ends at 0.0000.
    ///
    /// **`.land`, where it draws nothing at all and is still wrong.** Choosing
    /// a row has no momentum toward the row: the finger is resting on it.
    /// Traced, that animation carries literally nothing — `lift`, `reveal`,
    /// `cropped`, `trackX` and `pullOut` are all flat 0.0000 across it,
    /// because a landing happens with the finger ON the column, where
    /// everything `flatten` touches is already at rest, and the column's own
    /// furl runs on `syncMenu`'s spring rather than this one. It is changed
    /// anyway, because a spring that says "this arrived with momentum" on a
    /// gesture that had none is a claim the next person to add something to
    /// that arm inherits for free.
    ///
    /// **Everything else keeps `settle`, and that is checked rather than
    /// asserted.** A commit, a spring-back and a toggle all have a real throw
    /// behind them. `trackX` across a committed page turn peaks at
    /// **403.2226** — 1.2 points past the page it is turning to, which is the
    /// overshoot being earned — and it reads 403.2226 after this change too.
    static var settled: Animation { .spring(response: 0.3, dampingFraction: 1) }

    /// How long a finger may have been still and still count as moving.
    ///
    /// 60 milliseconds, which is nearly four frames at 60Hz and seven at 120.
    /// A finger that is still travelling produces an event every frame, so the
    /// gap between its last movement and its release is one frame; a finger
    /// that has stopped produces nothing at all. The two are separated by an
    /// order of magnitude and this sits between them, near the slow end so
    /// that a thumb crawling the last few points of a deliberate drag is read
    /// as the standstill it nearly is rather than as a throw. See
    /// `lastMoved` for the measurements this exists because of.
    static var stillFor: TimeInterval { 0.06 }

    /// On release, when what moved was the WHOLE screen.
    ///
    /// Slower and more damped, and that is the point rather than a taste: the
    /// two horizontal gestures differ in how much of the screen answers them,
    /// and a heavier thing that settles at exactly the speed of a lighter one
    /// stops feeling heavier the moment your finger leaves it. Half of the
    /// weight is what moves; this is the other half.
    static var settleAcross: Animation { .spring(response: 0.42, dampingFraction: 0.86) }

    /// A workspace growing out of its cell to fill the screen.
    ///
    /// Its own spring rather than `settleAcross`, which is a CROSSING — two
    /// cards passing each other, and a different response is right for a
    /// different distance.
    ///
    /// **Fully damped, because nothing that runs it has momentum toward it.**
    /// This was 0.74 — the bounciest spring in the shell — on the argument
    /// that something small becoming the whole screen "wants to arrive with a
    /// bit of spring in it". Every gesture that reaches it says otherwise:
    /// `open(at:)` is a tapped card, `honorRequest` is a notification, and
    /// `closeOverview` is the `Done` button. A tap has no momentum in the
    /// direction of the presentation, which is exactly the case WWDC 2018 803
    /// picks out — the Music app's minibar presents Now Playing at 100%
    /// damping for tapping it and 80% for swiping it away, and this was the
    /// tap wired to the swipe's number.
    ///
    /// What it drew was not subtle, and it was read off the frames rather than
    /// argued: `cropped` ran 1 → 0 and dipped to **−0.026** for eighteen
    /// frames before coming back. `ShellFlight.scale` turns that into
    /// **1.0152** — a tapped card grew into a page one and a half percent
    /// LARGER than the display, hung there for three hundred milliseconds with
    /// its right and bottom edges off the glass, and settled back. At 1.0 the
    /// same trace is monotone and lands on 0.
    static var settleOpen: Animation { .spring(response: 0.34, dampingFraction: 1) }

    /// The moment the page and its card change places.
    ///
    /// Short, and a fade rather than a spring, because nothing MOVES here: the
    /// page has already arrived at the cell and the two are the same rectangle
    /// in the same place. What crosses is only what is drawn inside it — a
    /// workspace's terminal for a workspace's name and tail — and a spring on
    /// a thing that is not travelling reads as a stutter at the end of a
    /// flight that had none.
    ///
    /// **One-sided.** Only the page's own alpha is on this animation; the card
    /// underneath is already opaque before it starts and is still opaque when
    /// it ends. A page and a card are not the same aspect ratio and never can
    /// be — that is what the crop is for — so this crossfade is real and
    /// cannot be argued away. What can be taken away is the DIP: two layers
    /// crossing at 50% each show the ground between them, and a rectangle that
    /// goes momentarily see-through in the middle of a landing is what reads
    /// as vanishing. Fading one opaque thing over another opaque thing has no
    /// such moment.
    static var handover: Animation { .easeInOut(duration: 0.16) }

    /// Whether the page is between the screen and its cell in either
    /// direction, which is the whole of when it is drawn over the grid.
    var flying: Bool { flights > 0 }

    /// The cell this workspace's page belongs in, or nil when the grid has not
    /// laid one out — a search that filters the current workspace out, most
    /// obviously — in which case the page simply does not fly.
    var tile: CGRect? { tiles[position.workspace] }

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
        // Two readers, and the nesting is the point.
        //
        // The shell is laid out FULL BLEED — a workspace fills the screen, so
        // the stack it lives in has to be the screen — but the bar and the
        // overview still have to clear the status bar and the home indicator.
        // `ignoresSafeArea` on a view zeroes the insets its own reader would
        // report, so one reader cannot answer both questions: the outer one is
        // asked where the safe area is, the inner one is asked how big the
        // screen is, and each is measured somewhere it can still tell the
        // truth. What comes back is then handed to the two layers that want it
        // as ordinary padding, which is the same inset spelled out where you
        // can see it rather than applied invisibly to everything.
        GeometryReader { safe in
            let insets = safe.safeAreaInsets
            GeometryReader { full in
                shell(page: full.size.width, safeArea: insets)
            }
            .ignoresSafeArea()
        }
        // The KEYBOARD is not furniture, and the outer reader must not report
        // it as any.
        //
        // SwiftUI implements keyboard avoidance as a safe-area inset, so a
        // `GeometryReader` that has not opted out reports `safeAreaInsets`
        // grown by the whole keyboard the moment one is up. That inset is the
        // number `chrome.bottom` and `barTrack`'s padding are both built from,
        // so with a keyboard on screen the shell's bar climbed 300 points up
        // the display and every pane was told the shell's furniture was that
        // tall — which is the exact opposite of what this shell promises
        // (`ShellPaneTrack.swift:55-61`: the bar and the track never move under
        // a keyboard, because the composer that rises with it lives in the
        // keyboard's own window).
        //
        // For the terminal it was worse than a moved bar, because
        // `TerminalView` subtracts the keyboard itself — deliberately, since
        // the host takes no automatic avoidance — so the room was taken TWICE
        // and the grid came out 0 points tall: a pane that rendered nothing,
        // and three UI tests that could not swipe an element with no visible
        // frame. Opting the reader out here leaves exactly one subtraction, in
        // the one view that knows how many rows it is losing.
        .ignoresSafeArea(.keyboard, edges: .bottom)
    }

    private func shell(page: CGFloat, safeArea: EdgeInsets) -> some View {
        // Back to front, and the order is the whole reveal.
        //
        // The overview is UNDER the page, not over it. Drawn on top it
        // composites its cards into the page as legible text — a workspace
        // name ghosted across a terminal, which is the double exposure
        // `ShellOverview`'s own ground is there to prevent and which no amount
        // of opacity fixes, because the page is what you are still reading.
        // Underneath, the same opacity is a REVEAL: what shows is what the
        // shrinking page has stopped covering, which is what an app switcher
        // shows you and the reason it never looks like a crossfade.
        ZStack(alignment: .bottom) {
            // Mounted from the first point of lift rather than from the point
            // where it starts to show. The page flies into a tile, so it needs
            // to know where that tile IS before it starts moving; a grid that
            // only appeared once the page was already travelling would leave
            // the first stretch of every lift with nowhere to fly to and the
            // page frozen until the geometry landed. The column phase is now
            // the whole of that head start, which is the one good thing about
            // a page that holds still for it.
            //
            // And kept mounted while a flight is in the air the OTHER way. The
            // page grows out of a cell on the way back, and a grid that
            // vanished on the first frame of that would leave it growing out
            // of nothing.
            // `reveal` and not `lift` is what carries this through a
            // release. `rest()` zeroes the lift unconditionally the instant
            // the finger leaves, and for one render that used to leave every
            // term of this condition false — so the grid was removed from the
            // tree and re-inserted on the next frame, fading in from nothing
            // underneath the flight.
            //
            // **`gestureActive && track == .bar` and not `lift > 0`, and
            // the difference is a frame the thumb is standing still in.**
            //
            // The head start was right. What was wrong was WHEN it was taken:
            // `lift > 0` is first true on the frame the thumb starts MOVING,
            // so a `NavigationStack`, a `ScrollView`, a `LazyVGrid` and its
            // first row of cards were all built in the middle of the tracking.
            // Measured off a `CADisplayLink` through a driven drag, on a Debug
            // simulator build: a bar drag lost **47 to 133 milliseconds** on
            // one frame six frames into the gesture, on every lift, in a fleet
            // of ten — 65 to 155 in a fleet of forty, so nearly all of it is
            // fixed cost and very little of it is the cards. A content drag
            // over the same panes never left 17. Taking the term out entirely
            // dropped every one of those frames to 17, which is how we know
            // this and not the pane track was where the cost was.
            //
            // `track` is set to `.bar` by `begin(on:)` on the first
            // `onChanged`, which a `DragGesture(minimumDistance: 0)` delivers
            // at touch DOWN — before the thumb has gone anywhere. So the same
            // work now lands on the frame the finger arrives, and the drag
            // that follows it tracks at 17 milliseconds a frame from its first
            // moving frame to its last: measured 43,20,17,17,17… where it used
            // to be 17,17,17,17,17,17,133,17. The work did not get cheaper. It
            // moved to the one frame in the gesture where nothing is being
            // drawn to a finger's position yet, which is the only frame in it
            // that can afford the work.
            //
            // The other half of why it is here and not on a `.task` at launch:
            // mounting the grid permanently is faster still — every lift then
            // costs 17 — but a grid in the tree at rest is a grid in the
            // ACCESSIBILITY tree at rest, and that was measured too: forty
            // static texts and every card's button, behind the workspace
            // somebody is actually in. Gating that on `overview` the way hit
            // testing already is destabilized the UI suite in a way this lane
            // could reproduce and not explain, so the version that keeps the
            // grid off the screen when nothing is touching it is the one that
            // ships.
            if (gestureActive && track == .bar) || lift > 0 || reveal > 0 || overview || flying {
                ShellOverview(
                    fleet: fleet, current: position.workspace,
                    // The cell the page is going to is a HOLE while it is on
                    // its way, and the page is what fills it when it lands.
                    //
                    // The grid reserves the space and draws nothing in it.
                    // Drawing a finished card there and flying a second copy
                    // of the same workspace on top of it is two of one thing,
                    // and it is what stops the lift reading as picking the
                    // screen up: a grid of workspaces where the one you were
                    // in is the one in your hand only works if it is in
                    // exactly one place at a time.
                    currentIsEmpty: cellIsHole,
                    // The chrome arrives with the DESTINATION, not with the
                    // gesture, and `overview` is exactly the moment the
                    // destination becomes one: it is false for every point of
                    // a lift, however far, and a release sets it. See
                    // `ShellOverview.chrome` for what the three pieces of
                    // chrome each cost, and why none of them costs the grid a
                    // point of layout.
                    //
                    // The same flag that gates hit testing below, and that is
                    // not a coincidence worth removing: a surface you cannot
                    // touch yet is a surface that has not arrived, and the
                    // header, the `Done` and the search field are the three
                    // things that claim it has.
                    chrome: overview,
                    liveServer: liveServer,
                    elsewhere: elsewhere,
                    search: $overviewSearch,
                    onOpen: open(workspace:),
                    onCross: onCross,
                    onDismiss: closeOverview,
                    // The tracked way out. `ShellOverview` reads the finger
                    // and decides nothing; these two spend it. See
                    // `ShellDrag.overviewPulled`.
                    onPull: overviewPulled,
                    onPullEnded: overviewPullReleased,
                    actions: overviewActions)
                    // Revealed over exactly the stretch where the page is
                    // moving to reveal it, which is the run past the last row.
                    // For the whole column phase the page is still and opaque
                    // and this is behind it, so anything else here would be a
                    // fade nobody can see.
                    .opacity(reveal)
                    .onPreferenceChange(ShellTileFrame.self) { tiles = $0 }
                    .allowsHitTesting(overview)
                    // The overview is a surface you read and type into, so it
                    // takes the safe area back. Its own ground bleeds past
                    // this — see `ShellOverview` — which is what keeps the
                    // darkening behind the lifted page edge to edge while the
                    // words inside it stay clear of the clock.
                    //
                    // `safeAreaPadding` rather than `padding`, and the
                    // difference is the whole reason the grid reads as a
                    // native screen. Plain padding makes the overview a
                    // smaller rectangle inside the display, so its navigation
                    // bar starts BELOW the clock and its content stops there
                    // too — a strip of bare ground above the chrome, and cards
                    // that vanish at a hard edge instead of sliding under the
                    // bar. Adding the inset to the SAFE AREA instead leaves
                    // the surface full bleed and tells the things inside it
                    // where the display's furniture is, which is what a
                    // navigation stack and a scroll view each want to know:
                    // the bar draws its material all the way up behind the
                    // clock, and the grid scrolls underneath it.
                    .safeAreaPadding(.top, safeArea.top)
                    .safeAreaPadding(.bottom, safeArea.bottom)
            }

            pageLayer(page: page, safeArea: safeArea)

            barTrack(page: page, safeArea: safeArea)
        }
        // Bottom-aligned, and spelled out on the frame as well as on the
        // stack. The page fills this stack and the bar does not, so the
        // alignment is the only thing saying where the bar goes; left at the
        // frame's default it would centre, and the bar would sit halfway up
        // the workspace.
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
        .background { screenFrameReader }
        .overlay(alignment: .topLeading) { probe }
        // One announcement per arrival. `onAppear` as well as `onChange`,
        // because the first pane the shell opens on is one nobody moved to and
        // is still the pane being read.
        .onAppear {
            onRest?(position)
            // On appear as well as on change, and the difference is a cold
            // launch. A card tapped before this app was running delivers its
            // URL, the connection answers, and the shell is mounted with the
            // request ALREADY set — so there is no change for `onChange` to
            // see, and the tap would open the app onto whatever it would have
            // opened onto anyway. Which is the failure the deep link exists to
            // remove. See `FleetView.dropUnknownTerminal`.
            honorRequest()
        }
        .onChange(of: position) { _, at in onRest?(at) }
        .onChange(of: request) { _, _ in honorRequest() }
    }

    /// The screen's frame, which is also the page's: everything in here is
    /// laid out full bleed, so this reader needs nothing to reach the edges.
    ///
    /// Measured OUTSIDE the page rather than inside it. A `GeometryReader`
    /// within the transformed subtree would be reporting a frame that the
    /// transform it is feeding had already moved, which is a measurement
    /// chasing its own answer.
    private var screenFrameReader: some View {
        GeometryReader { geo in
            Color.clear
                .onAppear { pageFrame = geo.frame(in: .global) }
                .onChange(of: geo.frame(in: .global)) { pageFrame = $1 }
        }
    }

    /// Which three are ON the track depends on the track the gesture started
    /// on: from the bar they are the adjacent WORKSPACES on the tab you last
    /// had open in each, and from the content they are the adjacent TABS along
    /// the flat sequence. Both neighbours are genuinely mounted and drawn at
    /// 0.72, which is what makes the incoming pane real rather than something
    /// that appears on commit.
    ///
    /// Everything about how they are mounted, retained and placed lives in
    /// `ShellPaneTrack`, and its header is where the argument is. The short
    /// version: this used to be a three-slot `HStack`, which keys its children
    /// by POSITION, so re-seating `position` destroyed the pane at slot 1 and
    /// the already-mounted pane at slot 2 along with it. Every commit rebuilt
    /// every pane.
    func paneTrack(page: CGFloat, safeArea: EdgeInsets) -> some View {
        ShellPaneTrack(
            fleet: fleet, position: position, previous: previousStep, next: nextStep,
            page: page, trackX: trackX, crossing: crossing,
            // What the shell puts over a pane: the display's furniture at the
            // top, and at the bottom the home indicator plus the bar and its
            // breathing room — the same two numbers `barTrack` pads by, said
            // once here so the two cannot drift.
            chrome: EdgeInsets(
                top: safeArea.top, leading: 0,
                bottom: safeArea.bottom + ShellMetrics.barRow + barGap, trailing: 0),
            pane: pane
        )
        .environment(\.shellDragClaim, dragClaim)
        // The half of `chrome.bottom` that is the DISPLAY's and not the
        // shell's, handed down so a pane can tell the two apart.
        //
        // `chrome.bottom` above is the home indicator plus the bar, as one
        // number, and that is right for a pane laid out in this shell's own
        // coordinates — `ShellHarness` insets by exactly it. A pane with a
        // `NavigationStack` in it is not laid out in those coordinates: the
        // framework re-derives the WINDOW's bottom inset on the far side of
        // the stack and applies it again, so a pane that then reserves
        // `chrome.bottom` on top has reserved the home indicator twice. Only
        // the shell knows which part of its own number the window is going to
        // give back, so it says so here. See `ShellPaneRealView.paneBottom`.
        .environment(\.shellDisplayBottom, safeArea.bottom)
        // **Alongside the pane, not behind it.**
        //
        // `.gesture` attaches at the LOWEST priority in SwiftUI: anything a
        // descendant declares wins the touch, and the shell never hears about
        // it. That was invisible for as long as the panes were a terminal —
        // whose scroll is a `UIPanGestureRecognizer` in UIKit's graph, not
        // SwiftUI's, and therefore not a competitor here at all — and a text
        // placeholder, which declares nothing. A diff declares something on
        // nearly every pixel, and the page turn simply stopped existing over
        // it. See `ShellDragClaim`, which has the measurement.
        //
        // `.simultaneousGesture` rather than `.highPriorityGesture`, and the
        // difference is the vertical half. High priority would win the drag
        // outright — including the plainly vertical ones this gesture reads
        // and then deliberately does nothing with — so a diff would stop
        // scrolling, which is the one thing that must stay perfect. Running
        // alongside leaves every pane's own gesture exactly as it was and only
        // stops the shell being cut out of the conversation.
        .simultaneousGesture(contentGesture(page: page))
    }

    private var previousStep: ShellStep? {
        fleet.step(from: position, .previous, along: track)
    }

    private var nextStep: ShellStep? {
        fleet.step(from: position, .next, along: track)
    }

    // MARK: - The bar, and the second weight

    /// Three bars, one page apart, on a track of their own.
    ///
    /// **This is the heavier of the two horizontal weights.** Within a
    /// workspace the bar holds perfectly still and only the pane slides: the
    /// workspace is not changing, and a bar that moved for every tab swipe
    /// would be saying something false twice a swipe. Across workspaces the
    /// bar travels a FULL PAGE, in step with the pane, so the whole screen
    /// leaves together and the neighbour's whole screen arrives — the way a
    /// browser changes tab.
    ///
    /// The rail used to do this from inside a single bar, sliding its contents
    /// by one rail width while the page moved by one page. It is the same
    /// information and it is a completely different sentence: something moving
    /// inside a surface that is itself still says "this bar is changing what
    /// it shows", and the whole surface leaving says "you are leaving". Only
    /// one of those is what a workspace change is, and only one of them can be
    /// told apart from a tab swipe with your eyes shut.
    private func barTrack(page: CGFloat, safeArea: EdgeInsets) -> some View {
        let width = ShellMetrics.railWidth(page: page)
        return HStack(alignment: .bottom, spacing: 0) {
            neighbourBar(previousStep, page: page, width: width)

            ShellBar(
                workspace: currentWorkspace,
                currentTab: position.tab,
                width: width,
                columnHeight: menuHeight,
                columnSelection: columnSelection)
                .accessibilityIdentifier("shell-bar")
                // Where the bar actually is, for the tap that chooses a column
                // row. See `barBottom`.
                .background { barFrameReader }
                .gesture(barGesture(page: page))
                .frame(width: page)

            neighbourBar(nextStep, page: page, width: width)
        }
        .frame(width: page * 3)
        .offset(x: barX)
        .frame(width: page)
        // The bar's own inset, plus the home indicator the stack around it no
        // longer reserves.
        .padding(.bottom, safeArea.bottom + barGap)
        // All the way to nothing, and NOT the prototype's `1 - overP * 0.9`.
        //
        // That residual tenth is right in the web version and wrong here, and
        // the difference is the z-order. The prototype composites the overview
        // ON TOP of everything, so a bar left at 10% sits UNDER a 94% ground
        // and nets about half a percent — invisible. Here the overview is
        // deliberately UNDERNEATH the page, because that is what makes the
        // lift a reveal rather than a crossfade (see `shell`), and a layer
        // that is on top does not get covered by anything: the last tenth of
        // the bar was being drawn straight over the bottom row of cards, where
        // a workspace's name and its ribbon were legible across a card that
        // names a different workspace. No amount of ground fixes that, because
        // the ground is behind the bar, not in front of it — the number that
        // was wrong is this one. Content beneath glass is matte; a card is
        // content; so the bar has to be GONE by the time the grid has arrived,
        // and `reveal` is exactly when it has.
        .opacity(1 - reveal)
        .allowsHitTesting(!overview)
    }

    /// The bar's bottom edge, reported out of the layout that draws it.
    ///
    /// Only `maxY`, and only when it changes. The frame's HEIGHT springs for
    /// the whole of the menu opening; its bottom edge does not move at all, so
    /// watching the one number costs a write when the safe area changes and
    /// nothing on any other frame.
    private var barFrameReader: some View {
        GeometryReader { geo in
            Color.clear
                .onAppear { barBottom = geo.frame(in: .global).maxY }
                .onChange(of: geo.frame(in: .global).maxY) { barBottom = $1 }
        }
    }

    /// The workspace waiting off one edge, drawn on the tab you would land on.
    ///
    /// Not hit-testable and not in the accessibility tree: there are three
    /// bars on this track and exactly one of them is the bar. A neighbour that
    /// answered to `shell-bar` would be the one a test found first, and it is
    /// a page off the side of the screen.
    @ViewBuilder
    private func neighbourBar(_ step: ShellStep?, page: CGFloat, width: CGFloat) -> some View {
        ShellBar(
            workspace: step.flatMap { workspace(at: $0.position.workspace) },
            currentTab: step?.position.tab ?? -1,
            width: width)
            .frame(width: page)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
    }

    /// How far the bar track has moved: the whole page, or nothing at all.
    ///
    /// Nothing at all unless the swipe will actually change workspace, which
    /// is what makes the two weights two weights. Reading the direction off
    /// `trackX` rather than latching it when the axis is decided keeps this
    /// continuous through a drag that reverses: the answer only changes as
    /// `trackX` passes zero, and at zero both answers are zero.
    ///
    /// A carried LIFT moves it too, and by the same rule rather than as a
    /// special case: a page held off the display and moved sideways is asking
    /// for the next workspace, so the whole screen leaves together — the bar
    /// included — and the neighbour's bar comes in behind it saying which
    /// workspace the card is being handed to. What would be strange is the
    /// other way round: the thing under your thumb sliding a third of the way
    /// across the display while the surface it came off sits perfectly still
    /// at seven tenths opacity.
    /// The breathing room under the bar, above the home indicator.
    ///
    /// Named because it is used twice and the two uses must agree: the bar is
    /// padded by it, and every pane is told the bar is this far up. A literal
    /// in both places is a pane whose last line sits under the glass the day
    /// somebody nudges one of them.
    private var barGap: CGFloat { 12 }

    private var barX: CGFloat {
        if carryX != 0 { return carryX }
        return crossesWorkspace(trackX) ? trackX : 0
    }

    /// Whether a sideways drag of `dx` would leave this workspace.
    ///
    /// The gate on both of the things that make a crossing heavier than a tab
    /// swipe: the bar travelling with the page, and the page becoming a card.
    /// Read off the translation rather than latched when the axis is decided,
    /// so it stays continuous through a drag that reverses — the answer only
    /// changes as the translation passes zero, and at zero both answers are
    /// zero.
    func crossesWorkspace(_ dx: CGFloat) -> Bool {
        guard let direction = ShellGesture.direction(dx: dx),
            let step = fleet.step(from: position, direction, along: track)
        else { return false }
        return step.crossesWorkspace
    }

    /// How much of a card a sideways drag of `dx` has made the pages, 0…1.
    func crossProgress(_ dx: CGFloat) -> CGFloat {
        crossesWorkspace(dx) ? ShellFlight.cardness(travel: abs(dx)) : 0
    }

    /// How much of the menu is showing: all of it, or none.
    ///
    /// **It POPS shut at the line where the page starts moving, rather than
    /// furling with the finger.** Past the last row there is no row left to
    /// choose, so the menu has stopped being relevant at exactly that point —
    /// and a menu that closes at the speed of the drag from there on is a
    /// second thing answering the same movement as the page, competing with
    /// it for the same travel. Nothing about a lift past the last row is a
    /// question about tabs any more.
    ///
    /// It used to be the full height scaled by `1 - reveal`, which furled it
    /// over exactly the 76 points that carry the page. The morph is the same
    /// morph either way — the menu closes back into the bar it grew out of —
    /// it is only the clock that changes, and `ShellMotion.menu` is the clock.
    private var menuHeight: CGFloat {
        menuOpen ? ShellGesture.columnFull(tabCount: tabCount) : 0
    }

    /// Whether the menu ought to be showing, given everything else.
    ///
    /// **Travel opens the column; PLACE decides whether there is anything in
    /// it**, and the two are different numbers both as soon as a gesture has
    /// redirected and whenever it did not start at the bar's own top edge.
    /// `lift` is the travel this gesture has been given — net of whatever
    /// `ShellBarDrag` charged a handover — and `pageAbove` is where the
    /// thumb actually is above the bar, which is `ShellBarDrag.Frame.above`
    /// carried into a `@State`.
    ///
    /// Asking `pageIsHeld` about the travel was a menu that lied for one
    /// frame, and it was found by watching the frames rather than by reading
    /// the code. A gesture that swipes 150 points sideways and then turns
    /// upward hands over at 212 points of lift, and the charge puts the
    /// travel at exactly the column's last row — which is inside the column
    /// by a hair, so the menu sprang open with the thumb 80 points above
    /// every row in it, and shut again on the next frame. `ShellMotion.menu`
    /// is a 0.28-second spring: what that draws is a blink.
    ///
    /// The SAME lie is available a second way — the owner's report — for a
    /// gesture that never redirected at all: touch down low in the bar and
    /// the raw travel to the column's own run reads as more lift than the
    /// thumb has actually climbed, so this used to be told the page was held
    /// (and hide the menu) before the thumb had genuinely cleared the last
    /// row. `pageAbove` is a place rather than a travel and does not have
    /// that failure mode either way.
    private var menuShouldShow: Bool {
        guard !overview,
            !ShellGesture.pageIsHeld(up: pageAbove, tabCount: tabCount)
        else { return false }
        return ShellGesture.columnHeight(up: lift, tabCount: tabCount, pinned: columnPinned) > 0
    }

    /// Bring the menu into line with everything else, on the menu's own
    /// spring.
    ///
    /// Called from outside whatever animation just ran, never inside one, so
    /// the surface is opened and shut by a transaction that belongs to it. The
    /// guard is what makes it safe to call after every gesture event: a value
    /// that has not changed opens no transaction at all, so this runs at most
    /// twice per gesture however many times the finger moves.
    func syncMenu() {
        let wanted = menuShouldShow
        guard wanted != menuOpen else { return }
        // **The bounce is for the one direction that earned it**, and which
        // one that is, is a question about the finger rather than about the
        // menu. A column growing out of the bar under a thumb travelling up is
        // motion in the direction the gesture already has — the case WWDC 2018
        // 803 says to reward. A column that appears because the bar was TAPPED
        // has no momentum behind it at all, and a column that pops shut past
        // the last row is travelling DOWN while the thumb goes up.
        //
        // `ShellBar` clamps the height at both ends —
        // `max(0, min(columnHeight, fullColumnHeight))` — so an unearned
        // overshoot mostly draws nothing, and that is the reason to be precise
        // about what it does draw rather than a reason to leave it. Traced
        // frame by frame, the close rings through zero: −5.07 points, back up
        // through **+0.195 for nineteen frames**, and down again. Height is
        // clamped, but `columnHeight > 0` is not — it is what tells the bar
        // its menu is open, what turns the ribbon's mark row on, and what
        // feeds `menuMark` a selection. So for three hundred milliseconds
        // after the column has visibly gone, the bar was still being told it
        // was showing one.
        let earned = wanted && lift >= ShellMetrics.openMin
        withAnimation(earned ? ShellMotion.menu : ShellMotion.menuSettled) { menuOpen = wanted }
    }

    var tabCount: Int { fleet.tabCount(ofWorkspace: position.workspace) }

    var currentWorkspace: ShellWorkspace? { workspace(at: position.workspace) }

    private func workspace(at index: Int) -> ShellWorkspace? {
        fleet.workspaces.indices.contains(index) ? fleet.workspaces[index] : nil
    }

    /// Which column row is under the finger, or which one you are on when the
    /// column is held open.
    ///
    /// A pinned column highlights the CURRENT tab, which is the only thing it
    /// could be highlighting: nothing is being dragged, so there is no finger
    /// to follow, and a pinned column with nothing highlighted would be a list
    /// that had forgotten where you are.
    private var columnSelection: Int? {
        // **The finger first, wherever there is one on an open column**, and
        // the pinned default only where there is not.
        //
        // This branch is the whole of the touch-down feedback the column had
        // none of. A column held open by a tap has `lift == 0`, so without it
        // every frame of the next touch fell through to `position.tab` below —
        // the highlight stayed on the tab you were already on for the whole
        // press and jumped to the row you chose at the instant you let go,
        // which is the confirmation arriving without the acknowledgement.
        // Measured before the fix, with a thumb held on the row nearest the
        // bar while tab 0 was current: the row under the thumb stood 0.95
        // brightness levels above its neighbour, against the 24.74 the lit row
        // stood above it — the highlight had not moved at all. See
        // `ShellGestureTests.testAColumnRowLightsUpUnderAThumb`.
        //
        // Both halves are `ShellGesture.columnRow`'s, so there is still one
        // mapping: it answers nil for a touch on the bar itself and nil for a
        // touch past the top of the column, and `fingerAbove` is nil whenever
        // no column is showing. So a finger dragged OFF the rows falls back to
        // the line below — which is the talk's cancel, drawn — and one that
        // comes back lights the row again.
        if let fingerAbove,
            let row = ShellGesture.columnRow(above: fingerAbove, tabCount: tabCount)
        {
            return row
        }
        if columnPinned && lift == 0 { return position.tab }
        // The same `openMin` a release is gated on, and then the same
        // `columnRow` a release resolves through — so what is lit is what
        // letting go would choose, by construction rather than by two
        // functions that happen to agree.
        guard lift >= ShellMetrics.openMin, let fingerAbove else { return nil }
        return ShellGesture.columnRow(above: fingerAbove, tabCount: tabCount)
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
                    + "pinned=\(columnPinned ? 1 : 0) overview=\(overview ? 1 : 0) "
                    // The tracked way out of the overview, in hundredths, so
                    // a test can ask how far a pull has got WHILE it is going
                    // rather than only what it resolved to. Nothing else in
                    // this string is a mid-gesture value, and this one has to
                    // be: the defect it exists to pin is a gesture that moved
                    // nothing until the release, and "it dismissed in the end"
                    // is exactly the assertion that passed all along.
                    + "pull=\(Int((pullOut * 100).rounded())) "
                    // The two mid-gesture numbers the arbitration is made of,
                    // and neither is legible any other way: how far the shell
                    // moved while a pane was using the same drag, and the one
                    // sample the content's axis was decided from. See
                    // `ShellRootView.strayed` and `ShellRootView.lockedOn`.
                    + "stray=\(Int(strayed.rounded())) "
                    + "lockx=\(Int(lockedOn.width.rounded())) "
                    + "locky=\(Int(lockedOn.height.rounded()))")
    }
}

/// A shell with nothing of its own to put in the overview's navigation bar.
///
/// `ShellHarness` stands the whole shell on a canned fleet, and a fixture has
/// no runner to switch to and no work to start — every one of the controls
/// `ShellScreen` contributes would be a button that could not do anything. The
/// grid, the search and `Done` are the platform's and are there either way.
extension ShellRootView where Actions == EmptyView {
    init(
        fleet: ShellFleet,
        initial: ShellPosition,
        openingOnOverview: Bool = false,
        request: Binding<String?> = .constant(nil),
        onRest: ((ShellPosition) -> Void)? = nil,
        liveServer: String? = nil,
        elsewhere: [ShellServerGroup] = [],
        onCross: @escaping (ShellServerGroup, ShellWorkspace) -> Void = { _, _ in },
        @ViewBuilder pane: @escaping (ShellPaneSlot) -> Pane
    ) {
        self.init(
            fleet: fleet, initial: initial, openingOnOverview: openingOnOverview,
            request: request, onRest: onRest, liveServer: liveServer, elsewhere: elsewhere,
            onCross: onCross, overviewActions: { EmptyView() }, pane: pane)
    }
}

extension EnvironmentValues {
    /// The display's own bottom inset — the home indicator — as the shell
    /// measured it, before the shell added its bar to it.
    ///
    /// Written by `ShellRootView` beside the pane track and read by the one
    /// kind of pane that needs it: one whose content is inside a
    /// `NavigationStack`, which is a `UINavigationController` and re-derives
    /// this same inset from the window on its own. Everything else in the
    /// shell wants `ShellPaneSlot.chrome`, which already includes it.
    ///
    /// Zero outside the shell, which is the honest answer for a pane mounted
    /// somewhere with no shell furniture over it.
    @Entry var shellDisplayBottom: CGFloat = 0
}


