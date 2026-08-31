import SwiftUI

// The finger, and what letting go of it means.
//
// Two `DragGesture`s, one axis lock, and the six things a release can be. It
// was all inside `ShellRootView` and is here now for the reason the file split
// at all: reading a finger and drawing a shell are two jobs, and a type that
// does both is a type where a change to one is reviewed against the other by
// accident.
//
// Nothing here DECIDES anything. Every threshold is `ShellGesture`'s and every
// release is `ShellFleet.barRelease` / `contentRelease`, both in
// `AgentKit/ShellNavigation.swift` and both covered by `swift test`. What is
// here is the part a pure function cannot hold: the lifetime of the axis lock,
// which transaction each write belongs to, and the silent re-seat at the end
// of a commit.

extension ShellRootView {
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
    func barGesture(page: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .global)
            .onChanged { value in
                // The anchor the shrink is written against, taken once, on
                // the first frame of the gesture. `startLocation` is already
                // in the global space this gesture is measured in, so the only
                // conversion is into the page's own coordinates.
                begin(on: .bar, from: value.startLocation.x - pageFrame.minX)
                let dx = value.translation.width
                let up = -value.translation.height
                decideAxis(dx: dx, up: up)
                // NOT inside an animation, and that is the whole of "one point
                // of page for one point of drag".
                //
                // Every value written here is a continuous function of where
                // the finger is right now, so every one of them is already the
                // answer — there is nothing for a spring to interpolate
                // TOWARD except a target the finger has since left. Wrapped in
                // `Self.tracking`, as this was, `lift` reached the screen
                // through an `interactiveSpring`, which is a low-pass filter
                // on the input: the page settled about 44 points behind the
                // thumb for the whole of a 1300 pt/s lift and kept travelling
                // for ~90ms after the thumb stopped. That is the lag the owner
                // reported, and WWDC 2018 803 is unambiguous about it — "the
                // moment the touch and content stop tracking one-to-one, we
                // immediately notice it".
                //
                // The discrete things a lift changes are all animated
                // ELSEWHERE and on their own transactions, which is why there
                // is nothing left in here that wants easing: the column pops
                // whole on `ShellMotion.menu` through `syncMenu` below, and
                // every release spring is `apply`'s. What is left is the
                // tracked path, and the tracked path is not animated.
                switch axis {
                case .horizontal:
                    trackX = ShellGesture.translation(dx: dx, rubberBanding: rubberBands(dx))
                    crossing = crossProgress(trackX)
                case .vertical:
                    let up = max(0, up)
                    lift = up
                    reveal = ShellGesture.overviewProgress(up: up, tabCount: tabCount)
                    // The other axis, and only once there is something in
                    // your hand to move. The axis lock is NOT released
                    // here — this is still the vertical gesture — the
                    // lift simply owns both directions from the moment the
                    // page leaves the display, which is the decision
                    // written down at `ShellGesture.pageIsHeld`.
                    carryX =
                        ShellGesture.pageIsHeld(up: up, tabCount: tabCount)
                        ? ShellGesture.translation(dx: dx, rubberBanding: rubberBands(dx))
                        : 0
                case nil:
                    break
                }
                // Its own transaction, and now the only one here. See
                // `syncMenu`.
                syncMenu()
            }
            .onEnded { value in
                let decided = axis
                let dx = value.translation.width
                // The lift the page is actually AT, which it now is: `lift` is
                // written straight in `onChanged`, so this is the number on
                // screen rather than a spring's target the finger had never
                // visually reached. It used to decide a release against a lift
                // nobody had seen.
                let up = lift
                let openedBefore = wasOpen
                // Read BEFORE `rest()`, which zeroes the lift the column's
                // height is computed from. Everything this needs — the lift,
                // whether the column was pinned — is state the release is
                // about to spend.
                let row = decided == nil ? tappedRow(at: value.startLocation) : nil
                rest()
                apply(
                    fleet.barRelease(
                        axis: decided, dx: dx, up: up, at: position, tapRow: row),
                    dx: dx, page: page, wasOpen: openedBefore)
                syncMenu()
            }
    }

    /// Which column row a tap at `point` chose, or nil when there was no open
    /// column under it.
    ///
    /// **Measured against the bar's own bottom edge rather than against the
    /// layout that puts it there.** `barBottom` is read off the surface
    /// SwiftUI actually drew — see `ShellRootView.barTrack` — because the
    /// alternative is a second copy of `safeArea.bottom + barGap` here, which
    /// would be right until somebody changed a padding and would then send
    /// every tap one row off with nothing on screen to say why. The one number
    /// still written down is `ShellMetrics.barRow`, and it is the shared
    /// constant the bar is drawn at rather than a literal.
    ///
    /// The bar's bottom and not its top: the bar row's position is fixed and
    /// the column grows UP out of it, so the top edge is a number that
    /// animates and the bottom edge is one that does not.
    private func tappedRow(at point: CGPoint) -> Int? {
        guard barBottom > 0,
            ShellGesture.columnHeight(up: lift, tabCount: tabCount, pinned: columnPinned) > 0
        else { return nil }
        return ShellGesture.columnRow(
            above: (barBottom - ShellMetrics.barRow) - point.y, tabCount: tabCount)
    }

    /// The content's own swipe, along the flat sequence.
    ///
    /// **Still `minimumDistance: 0` with a real terminal under it, and that
    /// was measured rather than assumed.** The note that stood here said this
    /// would have to grow a minimum distance or hand the pane the touch first,
    /// because a terminal's own pan is its scrollback and
    /// `TerminalScrollTests` is the regression that says so.
    ///
    /// What a runner actually showed is that the conflict is real and lives on
    /// the OTHER side of it. A terminal's touches belong to a
    /// `UIPanGestureRecognizer` on its keystroke sink
    /// (`TerminalView.swift:953-980`), which is not part of SwiftUI's gesture
    /// graph at all — and it used to BEGIN on sideways drags and win them,
    /// then convert `translation.y` to zero lines and do nothing. A terminal
    /// was a pane you could swipe into and never swipe out of, silently.
    ///
    /// It is `.simultaneousGesture` that resolves this, not a refusal on the
    /// terminal's side. There WAS a `gestureRecognizerShouldBegin` that claimed
    /// to refuse sideways drags, and this comment used to point at it — but it
    /// was never called, because no delegate was ever set. It was proved dead,
    /// and deleted rather than wired up: switching it on would have made a
    /// scroll-killing rule real for the first time and newly killed a scroll
    /// whose first ten points lean sideways.
    ///
    /// Both directions are pinned by tests on a real runner:
    /// `testTheShellDoesNotStealTheTerminalsScroll` and
    /// `testTheShellStillTurnsThePageOverALiveTerminal` are the two opposite
    /// failures this sits between, and they fail one at a time. Both pass
    /// without the dead rule, which is how we know it was never arbitrating.
    ///
    /// **And a terminal turned out to be the easy pane.** The line that stood
    /// here said `.gesture` rather than `.highPriorityGesture` was right
    /// because "the pane still wins its own scroll", which was true and was
    /// the wrong reason: `.gesture` is SwiftUI's LOWEST priority, so a pane
    /// wins not only its scroll but every drag it declares any gesture over at
    /// all. A terminal declares none — its pan is UIKit's, in a different
    /// graph — and neither does a text placeholder, so both of the panes this
    /// was built against turned the page and the defect could not be seen. A
    /// diff declares one nearly everywhere and the page turn simply stopped
    /// existing over it. It is `.simultaneousGesture` now; the argument is at
    /// the call site in `ShellRootView.paneTrack`, and what a pane does when it
    /// genuinely wants the same drag is `ShellDragClaim`.
    ///
    /// `minimumDistance: 0` still, and zero rather than the axis lock's six so
    /// the lock stays the only thing deciding what a drag means.
    func contentGesture(page: CGFloat) -> some Gesture {
        // `.global` for the same reason the bar's is, kept the same here so
        // the two gestures cannot come to measure different things.
        DragGesture(minimumDistance: 0, coordinateSpace: .global)
            .onChanged { value in
                begin(on: .content, from: value.startLocation.x - pageFrame.minX)
                let dx = value.translation.width
                decideAxis(dx: dx, up: -value.translation.height)
                guard axis == .horizontal else { return }
                // The part of the drag the pane could not use, and the whole
                // of the arbitration.
                //
                // While there is room under the finger the pane is scrolling
                // and the shell holds perfectly still — `handoff` follows the
                // finger so that nothing accumulates — and the frame the room
                // runs out, `handoff` stops moving and every point after it is
                // the shell's. One expression, and the handoff is a
                // subtraction rather than a state change: no threshold to
                // cross twice, no way to be handed a page turn that starts a
                // hundred points in.
                let carried = carriedX(dx)
                // Unanimated, for the reason the bar's `onChanged` gives at
                // length: both of these are the finger's position arithmetic,
                // and a spring over them is lag over a page turn.
                trackX = ShellGesture.translation(
                    dx: carried, rubberBanding: rubberBands(carried))
                crossing = crossProgress(trackX)
            }
            .onEnded { value in
                let decided = axis
                // The same subtraction the release is measured against. A
                // drag the pane absorbed entirely arrives here as zero, which
                // `contentRelease` already reads as a spring-back — so there
                // is no arm for "the pane took it", because a drag the shell
                // never had is a drag of no distance.
                let dx = carriedX(value.translation.width)
                rest()
                apply(
                    fleet.contentRelease(axis: decided, dx: dx, at: position), dx: dx,
                    page: page, wasOpen: false)
                syncMenu()
            }
    }

    /// The first `onChanged` of a gesture, and only the first.
    private func begin(on which: ShellTrack, from originX: CGFloat) {
        guard !gestureActive else { return }
        gestureActive = true
        // A new finger, so nothing under it has spoken yet. Cleared HERE
        // rather than at the end of the last gesture: `minimumDistance: 0`
        // means this runs the moment a finger lands, before anything under it
        // can have scrolled, which is the only instant at which "no pane has
        // claimed any of this drag" is true by construction.
        dragClaim.room = .none
        handoff = 0
        track = which
        wasOpen = columnPinned
        liftOrigin = originX
    }

    /// How much of a horizontal drag of `dx` belongs to the SHELL, once the
    /// scroller under the finger has taken what it can use.
    ///
    /// **A nested horizontal scroll inside a horizontal pager, which is the
    /// standard shape and the one the diff turned out to be.** A hunk with a
    /// long line in it wants exactly the drag the page turn wants; the rule
    /// everywhere on this platform is that the inner one goes first and hands
    /// over at its edge, and this is that rule as arithmetic.
    ///
    /// `handoff` is where the pane stopped being able to help, and this is the
    /// function that moves it. It follows the finger for as long as there is
    /// room — so the shell sees zero and holds still — and freezes the moment
    /// there is none, so what the shell sees from then on is the travel PAST
    /// the edge and a page turn that starts from nothing rather than jumping
    /// to wherever the finger had got to.
    ///
    /// It never runs backwards. A finger that reverses finds room on the other
    /// side and the pane takes the drag again, which is what a carousel does
    /// too: you are scrolling the line back, not un-turning a page.
    private func carriedX(_ dx: CGFloat) -> CGFloat {
        let absorbed = dragClaim.room.absorbs(dx: dx)
        if absorbed > 0.5 {
            handoff = dx
            // Anything the track had already travelled belongs to the pane
            // after all — the room is reported a frame behind the finger, so
            // the first point or two of a drag over a long line can reach the
            // shell before the hunk has said anything. Put back rather than
            // left standing: a page parked two points off centre for the rest
            // of a read is a page nobody asked to move.
            //
            // **The one write in this file that is still animated, and the
            // only remaining use of `Self.tracking`.** Everything else in both
            // `onChanged`s is the finger's position and is now written raw —
            // see the bar's, at length — but this is not that. It is a
            // CORRECTION of a couple of points the shell should never have
            // taken, it is not a function of where the finger is, and the
            // finger is not moving the thing it moves. Snapped it reads as a
            // twitch; eased it reads as the pane taking its drag back, which
            // is what happened.
            if trackX != 0 || crossing != 0 {
                withAnimation(Self.tracking) {
                    trackX = 0
                    crossing = 0
                }
            }
            return 0
        }
        return dx - handoff
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
    /// The page falls back the same way, and by the same spring: `lift` is
    /// what holds it up, so letting go of the bar drops the screen back onto
    /// the display with whatever velocity the finger left it — SwiftUI hands
    /// the in-flight `interactiveSpring` over to this one rather than starting
    /// from a standstill.
    ///
    /// `trackX` is deliberately NOT reset here, and neither are `crossing`,
    /// `carryX` or `reveal`. They are the shell's translation and the shell's
    /// SHAPE rather than facts about the gesture, every arm of `apply`
    /// resolves all four exactly once, and zeroing them here would make a
    /// commit animate from the centre as a full-bleed page — the page jumping
    /// back and flattening before it goes. `reveal` in particular has to
    /// survive this call: it is what holds the grid in the view tree across
    /// the frame where the lift has been zeroed and the overview has not yet
    /// been opened.
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
            withAnimation(Self.settle) { flatten() }
        case .land(let tab):
            withAnimation(Self.settle) {
                position.tab = tab
                // Landing on a row is choosing from the column, so the column
                // has done its job. It furls whether it was pinned or dragged
                // — a tap-opened column that stayed open after a choice would
                // leave the chosen pane behind a list of its siblings.
                columnPinned = false
                flatten()
            }
        case .openOverview:
            flyToCell()
        case .carry(let step):
            flyToCell(carrying: step)
        case .toggleColumn:
            withAnimation(Self.settle) {
                columnPinned = !wasOpen
                flatten()
            }
        }
    }

    /// Everything the page's shape and position are made of, back to a
    /// full-bleed page on the display.
    ///
    /// Called INSIDE the animation of whichever arm of `apply` ran, never
    /// before the release has decided — see `rest()`. One function rather than
    /// four copies because the failure mode of a copy is silent: a new arm
    /// that forgets `reveal` leaves the grid mounted at half strength over a
    /// workspace nobody is looking at.
    private func flatten() {
        trackX = 0
        crossing = 0
        carryX = 0
        reveal = 0
        // Zero for every arm that does not fly, and written here rather than
        // left implicit: a page that never left the display is already 0, and
        // one that came back down from a carried lift has to be told.
        cropped = 0
    }

    /// The page lets go of the finger and flies into its cell.
    ///
    /// One spring, from wherever the page was last drawn — carrying whatever
    /// velocity the finger left on it — to the tile, and the destination only
    /// becomes a destination here. Everything the drag did stays where the
    /// drag left it; nothing about the cell reached backwards into it.
    ///
    /// The count goes up before the flight and comes down when it lands,
    /// because the grid is holding an empty cell open for the whole journey
    /// and the page is what is in the air above it.
    ///
    /// `carrying` is the sideways half of the same release: the page was moved
    /// far enough toward a neighbour to be handed to it, so the workspace is
    /// re-seated in the SAME animation that flies the card. Not silently, and
    /// this is the one re-seat in this file that is deliberately animated. The
    /// card that leaves your thumb is the one you were holding and the card
    /// that lands is the one you asked for, and those are different
    /// workspaces; on one spring the pane inside it dissolves from the first
    /// to the second while it travels, and the cell it came from fills back in
    /// behind it over exactly the same stretch. Re-seating it silently instead
    /// puts both of those changes in one frame at the instant of release —
    /// a card whose contents snap and a card that pops into an empty slot
    /// somewhere else in the grid.
    private func flyToCell(carrying step: ShellStep? = nil) {
        flights += 1
        withAnimation(Self.settle, completionCriteria: .logicallyComplete) {
            if let step { position = step.position }
            overview = true
            reveal = 1
            // The page becomes a card ON THIS SPRING and not before it. The
            // crop, the last of the shrink, the corner and the card's own face
            // all hang off this one number, so all four travel with the
            // translation instead of stepping the moment `flights` changed.
            cropped = 1
            columnPinned = false
            trackX = 0
            crossing = 0
            // Not read at all once `overview` is true — the offset takes its
            // tile branch — so this costs nothing to look at and leaves the
            // channel at rest for the next gesture.
            carryX = 0
        } completion: {
            land()
        }
    }

    /// The page is on its cell, and stops being the page.
    ///
    /// The order here is the whole of the fix for "it vanishes into the hole,
    /// then the card fades in", and it is three steps that must not be two.
    ///
    /// 1. The card becomes real, UNANIMATED. The page is opaque and sitting
    ///    exactly on the cell, so a card appearing underneath it is a change
    ///    nobody can see — there is no frame in which it is half there.
    /// 2. The flight is over, also unanimated, and for the same reason: the
    ///    only thing `flights` still gates is the grid's mount.
    /// 3. Only then does the page dissolve, over `handover`, revealing the
    ///    card that has been fully present underneath it the whole time.
    ///
    /// What it used to do was step 3 alone, with the card fading IN on the
    /// same 0.16 seconds. Two layers crossing at 50% each is a rectangle you
    /// can see the floor through, and the eye reads that as the page falling
    /// into the hole rather than as one thing becoming another.
    private func land() {
        var silent = Transaction()
        silent.disablesAnimations = true
        withTransaction(silent) { flights -= 1 }
        // A landing that has been overtaken hands nothing over.
        //
        // Tapping a card before the flight that opened the overview has
        // finished starts a second flight the other way, and this completion
        // still arrives — from an animation whose destination the shell has
        // already left. That is what the count is for, and it is why this
        // decides on the state rather than on having been called: with
        // anything still in the air, or the overview no longer where we are,
        // the page is not a card and must not be dissolved. It used to be
        // safe by accident, because the page's opacity was a ternary that
        // read `!overview` and could not be wrong; a stored alpha can be, and
        // the way it is wrong is a page faded to nothing over a workspace —
        // a blank screen with no gesture that brings it back.
        guard flights == 0, overview else { return }
        withTransaction(silent) { cellIsHole = false }
        withAnimation(Self.handover) { pageAlpha = 0 }
    }

    /// The page takes its cell back and grows into the screen.
    ///
    /// The reverse journey, and the handover happens at the START of it: the
    /// card hands the page back before the page moves, so what grows out of
    /// the grid is the workspace rather than a second drawing of it.
    private func flyOut(_ change: @escaping () -> Void) {
        flights += 1
        // A page growing back out of a cell has no finger on it, so it grows
        // about its own centre. Set before the animation and invisible there:
        // `overview` is still true, so the offset is taking its tile branch
        // and nothing reads this until the branch changes.
        liftOrigin = nil
        // The same handover, run the other way and with the same rule: the
        // arriving layer dissolves in over a departing layer that is still
        // FULLY drawn, and the departing one is taken away only once the
        // arriving one is opaque. The page starts at exactly the card's
        // rectangle, so for those 0.16 seconds the card is covered by the
        // thing fading in over it and the pair is never less than solid.
        withAnimation(Self.handover, completionCriteria: .logicallyComplete) {
            pageAlpha = 1
        } completion: {
            var silent = Transaction()
            silent.disablesAnimations = true
            withTransaction(silent) { cellIsHole = true }
        }
        withAnimation(Self.settleOpen, completionCriteria: .logicallyComplete) {
            // The same number the outbound flight rides, run backwards: the
            // card unwraps into a page — crop, scale, corner and face
            // together — over exactly the travel that carries it back to the
            // display. This read `isFlying` once, and because `flights` is
            // raised outside this animation the page grew into the bottom
            // third of the screen and the rest of it appeared in one frame at
            // the end.
            cropped = 0
            change()
        } completion: {
            // Unanimated: the page is full-bleed and opaque by now, so the
            // grid behind it is going away where nothing can see it go.
            flights -= 1
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
        withAnimation(
            step.crossesWorkspace ? Self.settleAcross : Self.settle,
            completionCriteria: .logicallyComplete
        ) {
            trackX = sign * page
            // The card grows back into the screen AS IT TRAVELS, in this same
            // animation — not afterwards.
            //
            // This used to unwind in the completion handler, on a second
            // spring, and the comment there claimed `.logicallyComplete` made
            // the two overlap into one motion. It does not, or not visibly:
            // what you see is the slide arriving, everything stopping, and
            // only then the shrunken page growing back. Two motions where the
            // gesture is one.
            //
            // Animating them together is also the honest description of what
            // is happening. The card becoming a screen again IS the arrival;
            // it is not a thing that happens to the card once it has arrived.
            crossing = 0
            // A commit can now also arrive from a LIFT — a page held off the
            // display and flicked sideways hard enough to change workspace
            // without going all the way into the overview. The page falls back
            // onto the display as it crosses, on this one spring, which is the
            // same rule the line above states for a crossing: the card
            // becoming a screen again is the arrival.
            carryX = 0
            reveal = 0
            cropped = 0
            // The column belongs to the workspace it lists, so a crossing
            // furls it. A swipe within one workspace leaves it alone: the same
            // list is still the right list.
            if step.crossesWorkspace { columnPinned = false }
        } completion: {
            // Only the re-seat is silent, and it is invisible for the reason
            // it always was: the pane that was arriving is already the pane in
            // the middle slot, so zeroing the translation moves nothing. The
            // shape is no longer a problem here because the shape finished
            // growing on the way in.
            var silent = Transaction()
            silent.disablesAnimations = true
            withTransaction(silent) {
                position = step.position
                trackX = 0
            }
        }
    }

    func open(workspace index: Int) {
        guard fleet.workspaces.indices.contains(index) else { return closeOverview() }
        // On the tab you last had open there, not on tab 0. Tapping a card is
        // going to a workspace by name, which is the same kind of arrival a
        // bar swipe is — see `ShellWorkspace.resume` for the line between that
        // and the content swipe's continuum.
        open(at: ShellPosition(workspace: index, tab: fleet.workspaces[index].resumeTab))
    }

    /// Leave the overview by growing the page out of one particular cell.
    ///
    /// Split out of `open(workspace:)` so a deep link takes exactly the same
    /// journey as a tap — see `honorRequest`. Two copies of this would be two
    /// answers to "where does the page come from", and the answer is the whole
    /// of what makes the flight read as the card you touched opening.
    private func open(at target: ShellPosition) {
        // Two turns, and the split is what makes the page grow out of the card
        // you TAPPED rather than out of the one you came from.
        //
        // The page is invisible while the grid holds its card, so re-seating
        // it on the tapped workspace costs nothing to look at — but a flight
        // interpolates from what was last DRAWN, and a re-seat in the same
        // update as the flight is never drawn at all. So it gets its own turn.
        //
        // This is not the frame-boundary guess `commit` refuses to make: there
        // is no animation being raced here, only a render being waited for,
        // and the worst a late turn can do is what this code did before it —
        // start the flight from the cell you came from.
        var silent = Transaction()
        silent.disablesAnimations = true
        withTransaction(silent) { position = target }
        DispatchQueue.main.async {
            flyOut {
                overview = false
                reveal = 0
                overviewSearch = ""
            }
        }
    }

    func closeOverview() {
        flyOut {
            overview = false
            reveal = 0
            overviewSearch = ""
        }
    }

    /// Go where something outside the shell asked to go, once.
    ///
    /// The only navigation in this file that no finger performed, and the only
    /// one that comes from outside: a tapped Live Activity card, arriving as
    /// `farcooler://terminal/<id>`. See `ShellRootView.request` for why it is a
    /// one-shot request rather than a binding to `position`.
    ///
    /// **Cleared before anything else can return.** Every guard below is a
    /// reason to do nothing, and a request left standing after one of them is a
    /// request that blocks the next card naming the same pane — the same trap
    /// the pane host's `honorRequest` cleared first for, and the reason it did.
    ///
    /// Two ways in, because there are two places the shell can be. From the
    /// overview it grows the page back out of the tapped workspace's cell,
    /// which is the same journey a tapped card makes — the deep link is a card
    /// tapped from outside the app, and it should not arrive differently. From
    /// a page it is a silent re-seat: no animation, because there is no gesture
    /// and nothing on screen moved toward it. That is the pane host's
    /// retarget, arrived at from the shell's side, and it is what keeps every
    /// mounted pane exactly where it was.
    func honorRequest() {
        guard let id = request else { return }
        request = nil
        guard let target = fleet.position(ofTab: id) else { return }
        if overview {
            open(at: target)
        } else if target != position {
            var silent = Transaction()
            silent.disablesAnimations = true
            withTransaction(silent) {
                position = target
                // The column lists the workspace it belongs to. A link that
                // crosses workspaces would otherwise leave it open over a list
                // of somebody else's tabs; one that does not still moves the
                // tab out from under the highlighted row.
                columnPinned = false
            }
        }
    }
}
