import SwiftUI

// The panes, mounted once and moved rather than rebuilt.
//
// ## The invariant this file is
//
// **A pane must never be rebuilt, and its host's identity must never change.**
// Four comments in this codebase exist because that was got wrong four
// separate ways:
//
// - The pane host, `WorkspaceView`, mounted a pane and never destroyed it,
//   because tearing one down costs a scroll position, a half-typed message and
//   a tmux renegotiation. The screen is gone and the rule is not: it is
//   `ShellRetainedPane` below.
// - A route whose LOOKUP drove the view structure threw every pane away the
//   moment the lookup stopped succeeding — `WorkspaceRoute`, which latched the
//   starting pane once for exactly this reason. There are no routes now, and
//   the same trap is here as `ShellPaneRealView`'s `init` latch.
// - `Connection.swift:178-206` — writing focus back into the navigation path
//   changed a path element's value, and SwiftUI is free to rebuild a
//   destination whose value changed. `lastFocus` is still a dictionary beside
//   the navigation rather than inside it, and still for that reason.
// - And the one this file fixes, which the round that built the shell wrote
//   down and left standing: **a three-slot `HStack` keys its children by
//   POSITION.** `ShellRootView` drew `HStack { slot(previous), slot(current),
//   slot(next) }` with each slot's content `.id(tab.id)`, and that is stable
//   only while nothing moves. The moment `position` is re-seated the middle
//   slot's id goes from A to B and the trailing slot's from B to C — so
//   SwiftUI destroys the subtree at slot 1 and builds a fresh one, and the
//   pane for B that was ALREADY MOUNTED one slot over is destroyed too. Every
//   commit rebuilt all three panes. With text placeholders that is invisible;
//   with terminals it is three ssh streams renegotiated per swipe, and the
//   `.carry` release did it in the middle of a flight animation so you would
//   watch it happen.
//
// So identity is decoupled from slot. The panes live in one non-lazy `ZStack`
// keyed by TAB ID, and which of them is on screen is an `offset` — a number,
// not a place in a container. Re-seating `position` moves numbers. Nothing is
// destroyed by a swipe, ever, and there is no arrangement of the fleet that
// can make one identity land on another's view.
//
// It is a `ZStack` of absolutely-placed panes rather than the literal `HStack`
// the mechanics brief describes because the two sequences the track walks are
// not one order. From the bar the neighbours are the adjacent WORKSPACES at
// their first tabs; from the content they are the adjacent TABS along a flat
// sequence through the whole fleet. One container cannot be laid out in both
// orders at once, and a container that RE-ORDERS to answer the gesture is the
// position-keyed rebuild again with more steps. What the brief is actually
// asking for — never lazy, never `TabView(.page)`, never recycled — is
// enforced here in the only way that survives both sequences: explicit ids on
// a container that mounts everything it holds.
//
// ## Exactly one pane is visible
//
// `DockedBar.swift:34-41` is the sharp edge. An input accessory lives in the
// KEYBOARD's window, so a pane that is merely hidden goes on holding first
// responder and goes on drawing its composer over whatever is on top.
// Mid-gesture there are always two panes partly on screen, and only the one AT
// REST may be `isVisible` — which is why visibility is read off `position`
// alone. `position` is re-seated exactly once per release, so there is no
// moment, dragging or settling, when two panes both think they are the pane.

/// One pane's place in the track, as the thing that draws it needs it.
///
/// A struct rather than four positional arguments because the last two are
/// both `Bool` and mean opposite kinds of thing: one is about where the pane
/// is going and one is about what it is allowed to do. A call site that
/// swapped them would compile.
struct ShellPaneSlot {
    var position: ShellPosition
    var workspace: ShellWorkspace
    var tab: ShellTab
    /// This pane belongs to a workspace other than the one at rest, so its
    /// title names that workspace: the crossing has to be visible while it is
    /// still abandonable.
    var isCrossing: Bool
    /// The room the shell's own furniture takes out of this pane.
    ///
    /// The display's safe area at the top, and at the bottom the safe area
    /// plus the bar. A pane is laid out FULL BLEED — a workspace fills the
    /// screen, which is the whole point of the shell — so this is not layout,
    /// it is the pane being told where the glass is, exactly the way the pane
    /// host it replaced told its panes where the navigation bar and the tab
    /// strip were. Applied as a `safeAreaInset` rather than as padding, a
    /// scroll view consumes it as a CONTENT inset: rows still travel behind
    /// the bar, they just do not come to rest under it.
    var chrome: EdgeInsets
    /// The pane at rest, and there is exactly one in the whole track.
    ///
    /// What it buys is a stream, a poll and a tmux size assertion — see
    /// `TerminalView` — and what it costs when two panes have it is two
    /// composers fighting over first responder. Never true of a neighbour,
    /// however much of it is on screen.
    var isVisible: Bool
}

/// How much room a pane's own sideways scroller has left, in points, either
/// side of what it is showing.
///
/// Both directions, because a drag has one and the answer differs: a hunk
/// scrolled to the end of a long line has nothing left going left and a whole
/// line's worth going right, and "at the edge" is only ever a fact about a
/// direction.
struct ShellSidewaysRoom: Equatable {
    /// Points available going back — toward the start of the line.
    var before: CGFloat = 0
    /// Points available going on — toward the end of it.
    var after: CGFloat = 0

    /// Nothing under the finger scrolls sideways at all, which is what a
    /// terminal, a text pane and the ground between a diff's cards all report.
    static let none = ShellSidewaysRoom()

    /// How much of a drag of `dx` this scroller can still absorb.
    ///
    /// Negative `dx` is a finger travelling left, which reveals what is AFTER
    /// the visible text; positive is the other way. Clamped at zero because a
    /// scroller past its edge is rubber-banding, and a rubber band is not room.
    func absorbs(dx: CGFloat) -> CGFloat {
        dx < 0 ? min(-dx, max(0, after)) : min(dx, max(0, before))
    }
}

/// What a pane says about the drag it is in the middle of.
///
/// **One question, asked of whatever is under the finger: is there anything
/// left to scroll sideways in the direction this drag is going?** The shell
/// subtracts whatever the answer is from the drag before turning a page with
/// it, so a pane that can use the gesture gets it, a pane that cannot never
/// sees it, and a pane that runs out mid-drag hands the rest over without the
/// finger having to lift. That is the same handoff a carousel inside a paging
/// view does, and it is the whole mechanism — there is no branch anywhere that
/// knows what a diff is.
///
/// It exists because the shell and a pane want the same touch for two
/// different reasons, and the two obvious resolutions are both wrong.
///
/// The page turn used to be an ordinary `.gesture` on the track, which in
/// SwiftUI means it LOSES to anything a descendant declares. A terminal
/// declares nothing — its scroll is a raw `UIPanGestureRecognizer` outside
/// SwiftUI's graph entirely — and a text placeholder declares nothing, so both
/// of the panes the shell was built against turned the page perfectly. A diff
/// declares one on nearly every pixel: a segmented control, a row that opens
/// the commit list, a card per file, a button per hunk, and — over the code
/// itself — a horizontal `ScrollView` per hunk, because a diff line is a line
/// and wrapping one breaks the only property a diff has. Measured on
/// `-shell-harness -shell-changes`: of twelve horizontal swipes down the pane,
/// nine did nothing at all. That is the owner's "the horizontal swipes aren't
/// working very well on the diff view" — not a pane that never turns, a pane
/// that turns only where nothing is drawn.
///
/// Running the page turn as a `.simultaneousGesture` fixes all twelve and is
/// the wrong fix on its own: over a long line it makes BOTH answer, so the
/// code scrolls under your thumb while the page turns out from under it, and
/// reading a line past seventy points changes tab. Hence the room, and hence
/// it being a quantity rather than a flag — a veto would keep the page from
/// ever turning over code, which is the state this started in.
///
/// **Reported rather than negotiated, and that is why there is no
/// `UIGestureRecognizerDelegate` here.** The recognizer-level version of this
/// is `require(toFail:)` on a `UIScrollView`'s own `panGestureRecognizer`,
/// which means either replacing a delegate UIKit owns or walking the view
/// hierarchy to find scrollers the shell never created — and neither can
/// answer "has it anything LEFT in this direction", which is the actual
/// question, without reading the scroll geometry anyway. SwiftUI hands that
/// geometry over in public API (`onScrollGeometryChange`), so the pane answers
/// for itself and the shell does arithmetic.
///
/// A reference type in the environment rather than a `@State` binding, because
/// what reads it is a gesture callback rather than a `body`: the shell asks
/// this question in the middle of `onChanged`, and a value that had to travel
/// back up through a render would answer a frame late, which on a gesture is
/// the frame that matters.
///
/// `@unchecked Sendable` and not `@MainActor`, because an `EnvironmentKey`'s
/// default value is built in a nonisolated context and a main-actor type
/// cannot be. Everything that touches it — a gesture callback and a scroll
/// callback — is already on the main actor.
final class ShellDragClaim: @unchecked Sendable {
    /// What the scroller under the finger has left. Reset by the shell when a
    /// finger goes down, so it never describes the drag before this one.
    var room = ShellSidewaysRoom.none
}

extension EnvironmentValues {
    /// Handed down by `ShellRootView` and written by any pane that scrolls
    /// sideways. Outside the shell it is an object nobody reads, which is
    /// exactly what a `ChangesView` mounted anywhere else should see.
    @Entry var shellDragClaim: ShellDragClaim = ShellDragClaim()
}

/// A pane the track is keeping alive, and why.
///
/// Keyed on the TAB's id and nothing else. Not on the index and not on the
/// position: an id that moves when the fleet reorders is an id that rebuilds a
/// pane the reorder did not touch.
private struct ShellRetainedPane: Identifiable, Hashable {
    /// The tab this pane is, for the whole of its life.
    let tab: String
    /// The workspace it belongs to, for the scope rule below. By id, because
    /// a workspace index is not stable across a poll.
    let workspace: String

    var id: String { tab }
}

/// The panes of a fleet, mounted, retained, and translated under a finger.
///
/// This is the pane host's `visited` / `select` / `prune` / `content(of:)`
/// lifted up to the shell, which is where they now belong and where they now
/// only exist: the shell spans the whole fleet, so the set of panes worth
/// keeping is no longer a property of one screen. The `.opacity` / `.allowsHitTesting` / `.id(pane.id)` triple is
/// carried across verbatim, because that triple IS the retention mechanism —
/// hidden and not removed, so the view stays in the hierarchy and its state
/// with it, and unable to swallow a touch meant for the pane on top of it.
struct ShellPaneTrack<Pane: View>: View {
    let fleet: ShellFleet
    /// The pane at rest. The one thing a commit re-seats, and the only source
    /// of "which pane is visible".
    let position: ShellPosition
    /// The two neighbours, along whichever sequence the gesture is walking.
    let previous: ShellStep?
    let next: ShellStep?
    /// One pane's width, which is the display's.
    let page: CGFloat
    /// The track's own translation, in points.
    let trackX: CGFloat
    /// How much of a card the panes have become for a sideways crossing, 0…1.
    let crossing: CGFloat
    /// The room the shell's own furniture takes out of every pane. See
    /// `ShellPaneSlot.chrome`.
    let chrome: EdgeInsets
    /// What a pane is. Handed a slot rather than built here, so this file
    /// never has to know what a pane is — the harness passes placeholders, the
    /// app passes terminals.
    let pane: (ShellPaneSlot) -> Pane

    /// Panes that have been AT REST at least once, oldest first.
    ///
    /// An array rather than a set so the order is stable — SwiftUI identity in
    /// a `ForEach` is the whole mechanism here, and a set's iteration order is
    /// not a thing to build a view tree on.
    ///
    /// Only panes actually visited are retained. Mounting every tab in the
    /// fleet would pay setup for panes nobody opens — including fetching a
    /// diff nobody asked to see, forty times — and a handful of visited panes
    /// is what "the tabs I am working in" actually means. The two NEIGHBOURS
    /// are mounted too, and are not in here: they are mounted because they are
    /// on screen during a swipe and released again when they stop being, which
    /// costs nothing because a pane that has never been at rest has never held
    /// a scroll position or a half-typed message to lose.
    @State private var retained: [ShellRetainedPane] = []

    var body: some View {
        ZStack(alignment: .topLeading) {
            ForEach(mounted) { entry in
                mount(entry)
            }
        }
        // A FIXED width, and this is not interchangeable with
        // `.frame(maxWidth: .infinity)`. A flexible frame's lower bound is its
        // CHILD's width when no `minWidth` is given, and the shell then becomes
        // wider than the screen, the bar centres itself off the right edge, and
        // the whole thing renders empty. A fixed frame is what a page is.
        .frame(width: page)
        .frame(maxHeight: .infinity)
        // An opaque ground, INSIDE the transform, and it is what makes the
        // shrink read as a page rather than as text getting smaller. The dim
        // over it is the desk the cards sit on, and it only exists while they
        // are crossing: each pane carries the same ground, so the gap a
        // shrunken card opens beside it would otherwise reveal the exact colour
        // the card had been covering.
        .background {
            ZStack {
                Themes.shared.current.backgroundColor
                Color.black.opacity(ShellMotion.deskDim * crossing)
            }
        }
        // And clipped, which is not optional once the page shrinks. The
        // neighbours sit exactly one page off each edge, so they are off the
        // display at rest — but a page scaled to a third of its size brings
        // them back inside it, and a shrinking screen with two ghost pages
        // beside it is not a screen.
        .clipped()
        .contentShape(.rect)
        // The retained set is state, so it is written in a change handler and
        // never in `body`. `mounted` below is what makes that safe: it is the
        // union of the retained set and the three slots, computed fresh every
        // pass, so a pane is on screen in the frame the gesture asks for it
        // rather than in the frame after.
        .onAppear { record() }
        .onChange(of: restingTab) { _, _ in record() }
        .onChange(of: fleetTabIDs) { _, _ in record() }
    }

    /// One pane, mounted, placed and weighted.
    ///
    /// Split out of the `ForEach` because the type checker gave up on it
    /// inline, and left as one function because the two branches are one
    /// decision: whether this pane is on the track this frame. Both of them
    /// carry the same `.opacity` / `.allowsHitTesting` / `.id` triple, which
    /// is the retention mechanism and is why a pane that is off the track is
    /// still a pane rather than a gap.
    @ViewBuilder
    private func mount(_ entry: ShellRetainedPane) -> some View {
        let placed = slot(of: entry)
        pane(placed?.slot ?? offTrack(entry))
            .frame(width: page)
            .frame(maxHeight: .infinity)
            // Each pane is a card, and the card treatment lives HERE rather
            // than on the track as a whole. The whole point of a crossing is
            // that there are two of them: rounding the track would round the
            // SCREEN and leave two square pages sliding behind one rounded
            // window, which is a window, not a pair of cards.
            //
            // At rest all three of these are identities: a radius of zero, a
            // scale of one, and a ground the same colour as the one behind it.
            // That is what keeps a tab-to-tab swipe inside a workspace exactly
            // as light as it was — `crossing` is zero for one of those, and so
            // is all of this.
            .background(Themes.shared.current.backgroundColor)
            .clipShape(
                RoundedRectangle(
                    cornerRadius: ShellMotion.screenCorner * crossing, style: .continuous))
            .scaleEffect(1 - (1 - ShellMotion.crossingScale) * crossing)
            // Where the pane IS, which is the whole of what a commit changes.
            // The prototype's `-PAGE_W + dx`, said once per pane instead of
            // once for a container whose children have to be kept in step with
            // it.
            //
            // A pane that is not on the track sits at zero and is invisible.
            // Parked there rather than off the edge because a hidden pane that
            // MOVES is a hidden pane whose scroll view re-lays-out for nothing.
            .offset(x: placed.map { CGFloat($0.rank) * page + trackX } ?? 0)
            // Hidden, not removed. `opacity` keeps the view in the hierarchy —
            // which is what preserves its state — while `allowsHitTesting`
            // stops a pane nobody can see from swallowing taps meant for the
            // one on top of it.
            .opacity(placed?.weight ?? 0)
            .allowsHitTesting(placed?.rank == 0)
            // Never recycled onto a different pane.
            .id(entry.id)
    }

    // MARK: - Which panes are mounted

    /// The pane at rest, by id, or nil when the fleet no longer has one there.
    private var restingTab: String? { fleet.tab(at: position)?.id }

    /// Every tab the fleet currently holds, as something `onChange` can
    /// compare. Sorted, because `onChange` wants a stable order and a
    /// workspace gaining a terminal must not read as every pane moving.
    private var fleetTabIDs: [String] {
        fleet.workspaces.flatMap { $0.tabs.map(\.id) }.sorted()
    }

    /// The three positions the track is drawing, nearest neighbour first.
    private var slots: [(rank: Int, step: ShellStep?)] {
        [(-1, previous), (0, ShellStep(position: position, crossesWorkspace: false)), (1, next)]
    }

    /// Everything that has to be in the view tree this pass: the panes worth
    /// keeping, plus the panes the gesture is showing.
    ///
    /// Derived rather than stored, and that is what lets the retained set be
    /// written from a change handler without ever being a frame behind. A
    /// neighbour appears here the instant the gesture asks for it; whether it
    /// STAYS is `record()`'s business, one frame later, and by then it has
    /// either been landed on or not.
    private var mounted: [ShellRetainedPane] {
        var out = retained.filter(inScope)
        for slot in slots {
            guard let step = slot.step, let entry = entry(at: step.position) else { continue }
            guard !out.contains(where: { $0.id == entry.id }) else { continue }
            out.append(entry)
        }
        return out
    }

    private func entry(at position: ShellPosition) -> ShellRetainedPane? {
        guard fleet.contains(position), let tab = fleet.tab(at: position) else { return nil }
        return ShellRetainedPane(tab: tab.id, workspace: fleet.workspaces[position.workspace].id)
    }

    /// Whether a retained pane is still worth keeping mounted.
    ///
    /// Two rules, and they are the two ways a pane stops being worth its ssh
    /// stream.
    ///
    /// **It still exists.** This is the pane host's `prune` — a terminal the
    /// runner has forgotten cannot be the thing on screen, and a `TerminalView`
    /// left mounted for one would hold a session for a pane the host has no
    /// record of. `ShellFleet.position(ofTab:)` answering nil is the whole
    /// test.
    ///
    /// **It is near.** The workspace it belongs to is the one you are in or one
    /// of its two neighbours. Before the shell, leaving a workspace was popping
    /// a route, which destroyed every pane in it outright; keeping the
    /// neighbours means a swipe across a workspace boundary and straight back
    /// costs nothing, which is the gesture the shell has that the stack did
    /// not. Two workspaces further and the panes go, because forty workspaces
    /// of retained terminals is forty ssh streams and the reason a phone gets
    /// warm.
    ///
    /// **Never prune to nothing.** A poll that briefly returns an empty fleet —
    /// a reconnect, a runner mid-restart — would otherwise unmount every pane
    /// and throw away exactly the state this type exists to keep. The same
    /// guard, for the same reason, as that `prune` used.
    private func inScope(_ entry: ShellRetainedPane) -> Bool {
        guard !fleet.isEmpty else { return true }
        guard fleet.position(ofTab: entry.tab) != nil else { return false }
        return nearby.contains(entry.workspace)
    }

    /// The workspace at rest and its two neighbours, by id.
    private var nearby: Set<String> {
        let here = position.workspace
        return Set(
            (here - 1...here + 1)
                .filter { fleet.workspaces.indices.contains($0) }
                .map { fleet.workspaces[$0].id })
    }

    /// Remember the pane at rest, and forget the ones that have gone.
    ///
    /// This is the pane host's `select` and `prune` in one call,
    /// because in the shell they are answers to the same question — the fleet
    /// arriving and the position moving both change which panes are worth
    /// keeping, and running them apart is two chances to leave the set
    /// disagreeing with itself.
    ///
    /// Appended and never inserted, so a pane's place in the `ForEach` only
    /// ever moves when something before it is dropped. Its identity does not
    /// move at all.
    private func record() {
        var next = retained.filter(inScope)
        if let entry = entry(at: position), !next.contains(where: { $0.id == entry.id }) {
            next.append(entry)
        }
        guard next != retained else { return }
        retained = next
    }

    // MARK: - Where each pane is drawn

    /// A pane's rank in the track and the weight it is drawn at, or nil for a
    /// pane that is mounted but not on it.
    private func slot(of entry: ShellRetainedPane) -> (
        rank: Int, weight: Double, slot: ShellPaneSlot
    )? {
        for candidate in slots {
            guard let step = candidate.step, let tab = fleet.tab(at: step.position),
                tab.id == entry.id, let workspace = workspace(at: step.position)
            else { continue }
            // A tab inside this workspace comes in at 0.72 — real, mounted and
            // moving, but visibly not the page you are on yet. A tab in the
            // NEXT workspace comes in whole, because it is not arriving alone:
            // its bar is arriving beside it, at full strength, and a page at
            // 72% under a bar at 100% is two things sliding in rather than one
            // screen.
            let weight: Double =
                candidate.rank == 0 ? 1 : (step.crossesWorkspace ? 1 : 0.72)
            return (
                candidate.rank, weight,
                ShellPaneSlot(
                    position: step.position, workspace: workspace, tab: tab,
                    isCrossing: step.position.workspace != position.workspace,
                    chrome: chrome,
                    // Read off the RANK and nothing else. The pane at rest is
                    // the pane at slot zero, and `position` is re-seated
                    // exactly once per release — so there is no frame in which
                    // two panes answer true, however much of each is on screen.
                    isVisible: candidate.rank == 0)
            )
        }
        return nil
    }

    /// A retained pane that is not on the track: still mounted, drawn nowhere,
    /// and emphatically not visible.
    private func offTrack(_ entry: ShellRetainedPane) -> ShellPaneSlot {
        let at = fleet.position(ofTab: entry.tab)
        let workspace = at.flatMap(self.workspace(at:))
        return ShellPaneSlot(
            position: at ?? position,
            workspace: workspace
                ?? ShellWorkspace(id: entry.workspace, name: "", tabs: []),
            tab: at.flatMap(fleet.tab(at:)) ?? ShellTab(id: entry.tab, title: "", mark: .working),
            isCrossing: false,
            chrome: chrome,
            isVisible: false)
    }

    private func workspace(at position: ShellPosition) -> ShellWorkspace? {
        fleet.workspaces.indices.contains(position.workspace)
            ? fleet.workspaces[position.workspace] : nil
    }
}
