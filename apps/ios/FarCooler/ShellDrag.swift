import SwiftUI

// The finger, and what letting go of it means.
//
// Two `DragGesture`s, two different lifetimes for one axis rule, and the seven
// things a release can be. It was all inside `ShellRootView` and is here now
// for the reason the file split at all: reading a finger and drawing a shell
// are two jobs, and a type that does both is a type where a change to one is
// reviewed against the other by accident.
//
// Nothing here DECIDES anything. Every threshold is `ShellGesture`'s and every
// release is `ShellFleet.barRelease` / `contentRelease`, both in
// `AgentKit/ShellNavigation.swift` and both covered by `swift test`. What is
// here is the part a pure function cannot hold: how long an axis lasts — once
// for the content, a frame at a time for the bar — what a handover between the
// two of them puts back, which transaction each write belongs to, and the
// silent re-seat at the end of a commit.

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
    /// travelled — a column that stops at 30 points for a 60-point drag and a
    /// page that needs twice its documented reach. Nothing about it looks
    /// like a bug; it just feels heavy.
    func barGesture(page: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .global)
            .onChanged { value in
                // The anchor the shrink is written against, taken once, on
                // the first frame of the gesture. `startLocation` is already
                // in the global space this gesture is measured in, so the only
                // conversion is into the page's own coordinates.
                //
                // `startAbove` travels alongside it, for the same reason and
                // on the same frame: how far the TOUCH-DOWN point sat above
                // the bar's own top edge, in `columnAbove`'s own units. See
                // `ShellBarDrag.startAbove`'s header — this is where the
                // owner's report is fixed, by giving the redirection struct
                // the one fact about the touch it did not have before.
                begin(
                    on: .bar, from: value.startLocation.x - pageFrame.minX,
                    startAbove: aboveBarTopEdge(value.startLocation.y))
                noteMovement(value)
                barMoved(
                    dx: value.translation.width, up: -value.translation.height,
                    at: value.location)
            }
            .onEnded { value in
                let decided = barDrag.axis
                // Net of the handover, and for one reason: what a release
                // measures is what the shell has been DRAWING, and what it
                // has been drawing since the gesture changed its mind is the
                // travel since it changed its mind. `up` gets this for free —
                // `lift` is already net of `ShellBarDrag.spentLift` because
                // that is what was written to it — and `dx` has to be told.
                let dx = value.translation.width - barDrag.spentSideways
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
                //
                // For the DRAG as well as the tap, and that is the change:
                // a drag used to have its row derived inside `barRelease`
                // from the lift alone, which is a different mapping from this
                // one and disagreed with it by a whole row for any gesture
                // that did not start on the bar row's very top edge. One
                // mapping, one point, both gestures.
                //
                // `value.location` and not `startLocation`: a touch is
                // confirmed where it goes UP. For a tap the two are within
                // the six points the axis rule allows, so this is the same
                // row; for a drag it is the only one of the two that means
                // anything.
                let row = columnRow(at: value.location, lift: up)
                let thrown = releaseVelocity(value)
                // The finger's real place, read before `rest()` zeroes it —
                // the same reason `up` is read from `lift` a few lines up.
                // Handed to `barRelease` so a release reads the same place
                // the drag was drawing rather than falling back to `up`
                // alone, which would revive the low-in-the-bar defect at the
                // one moment a person actually lets go.
                let above = pageAbove
                rest()
                apply(
                    fleet.barRelease(
                        axis: decided, dx: dx, up: up, at: position, row: row, above: above,
                        // SwiftUI's velocity is points per second in the
                        // gesture's own space, `height` positive DOWN — so
                        // the lift's is negated, the same way `up` is.
                        dxVelocity: thrown.width,
                        upVelocity: -thrown.height),
                    dx: dx, page: page, wasOpen: openedBefore)
                syncMenu()
            }
    }

    /// One frame of the BAR's finger.
    ///
    /// A named function rather than the closure it was, for the reason the
    /// rest of this lane is: a redirection is a thing you have to WATCH, and
    /// a gesture callback is the one shape of code neither `swift test` nor a
    /// screenshot can reach. `ShellBarDrag` holds the arithmetic and this
    /// holds the transactions, and between them the only thing left in
    /// `onChanged` is where the numbers came from.
    private func barMoved(dx: CGFloat, up: CGFloat, at point: CGPoint) {
        // **One frame of the redirection, and the whole of it.** Which axis
        // this gesture is leaning toward NOW, what each channel is drawn at
        // once the handovers have been charged, and whether an axis has just
        // taken the gesture off the other. All of it is `ShellBarDrag`'s, in
        // AgentKit, under `swift test`; what is left here is the transaction.
        let frame = barDrag.moved(dx: dx, up: up, tabCount: tabCount)
        if let claimed = frame.claimed { handOver(to: claimed) }
        // NOT inside an animation, and that is the whole of "one point of page
        // for one point of drag".
        //
        // Every value written here is a continuous function of where the
        // finger is right now, so every one of them is already the answer —
        // there is nothing for a spring to interpolate TOWARD except a target
        // the finger has since left. Wrapped in `Self.tracking`, as this was,
        // `lift` reached the screen through an `interactiveSpring`, which is a
        // low-pass filter on the input: the page settled about 44 points
        // behind the thumb for the whole of a 1300 pt/s lift and kept
        // travelling for ~90ms after the thumb stopped. That is the lag the
        // owner reported, and WWDC 2018 803 is unambiguous about it — "the
        // moment the touch and content stop tracking one-to-one, we
        // immediately notice it".
        //
        // The discrete things a lift changes are all animated ELSEWHERE and on
        // their own transactions, which is why there is nothing left in here
        // that wants easing: the column pops whole on `ShellMotion.menu`
        // through `syncMenu` below, and every release spring is `apply`'s.
        // What is left is the tracked path, and the tracked path is not
        // animated.
        //
        // **Neither arm writes the other's channel.** What an abandoned
        // channel does is not a per-frame fact, it is the one-off apology
        // `handOver` makes on the frame the gesture changed its mind, eased
        // over `Self.tracking` for exactly the reason `carriedX` eases its own
        // put-back. Zeroing it again here every frame would snap that ease
        // flat.
        switch frame.axis {  // and not the view's `axis`, which is the content's
        case .horizontal:
            trackX = ShellGesture.translation(
                dx: frame.sideways, rubberBanding: rubberBands(frame.sideways))
        case .vertical:
            lift = frame.lift
            // WHERE THE FINGER IS, not how far it has come, because the row it
            // is choosing is the row drawn under it. See
            // `ShellRootView.columnSelection`, which is the highlight this
            // feeds, and `ShellGesture.columnRow`, which is the single mapping
            // both it and the release go through.
            //
            // Written on every frame and read by nothing else: the release
            // measures its own row off the point the finger came up at, so a
            // highlight and the tab you get can only ever disagree by the
            // frame the finger lifted in.
            //
            // **A place on the glass, so it survives a redirection
            // untouched.** A gesture that turns upward out of a sideways drag
            // has its lift charged for the page's travel —
            // `ShellBarDrag.spentLift` — and none of that reaches the
            // highlight: the row under your thumb is the row under your thumb
            // whichever axis owns the gesture and whatever the lift has been
            // charged. That is the property the previous lane established, and
            // it is what made unlocking the axis safe to attempt at all.
            fingerAbove = columnAbove(point, lift: frame.lift)
            // The finger's own place above the bar, carried into a `@State`
            // so `ShellPageLayer.pageRise` and `ShellRootView.menuShouldShow`
            // can read it outside this function. `frame.above` and not
            // `frame.lift` — see `ShellBarDrag.Frame.above`'s header — which
            // is the fix for a drag that starts low in the bar: `lift` alone
            // said the page should already be rising with the fingertip
            // still short of the topmost row.
            pageAbove = frame.above
            // The other axis, and only once there is something in your hand to
            // move. This is not a redirection — the gesture is still the
            // vertical one and `lean` has stopped being asked — it is the lift
            // owning both directions from the moment the page leaves the
            // display, which is the decision written down at
            // `ShellGesture.pageIsHeld`.
            carryX =
                ShellGesture.pageIsHeld(up: frame.above, tabCount: tabCount)
                ? ShellGesture.translation(
                    dx: frame.sideways, rubberBanding: rubberBands(frame.sideways))
                : 0
        case nil:
            // **No axis yet is not "nothing to draw" — it is TOUCH-DOWN**, and
            // it is the only frame a column held open by a tap ever gets.
            //
            // A pinned column has `lift == 0` and a finger that has not moved,
            // so it never reaches the vertical arm above and the highlight sat
            // on the tab you were already on until the finger LIFTED, and then
            // jumped. That is the shell's primary tab switcher answering a
            // touch with nothing, on the one surface where the answer is
            // already drawn and only needs pointing at. WWDC 2018 803 puts it
            // first among the things a tap owes: *"the button should highlight
            // immediately when I touch down on it… but we shouldn't confirm the
            // tap until my touch goes up."*
            //
            // `columnAbove` is the same guard and `ShellGesture.columnRow` the
            // same mapping the release resolves through, so this adds a
            // MOMENT, not a second answer: it is nil unless a column is
            // actually showing and nil unless the touch is on it, which leaves
            // an ordinary bar drag writing nothing here and a tap on the bar
            // itself the toggle it always was.
            //
            // `lift` and not a frame value: this arm runs before any axis has
            // been decided, so the shell is drawing whatever it was drawing —
            // for a pinned column, zero.
            fingerAbove = columnAbove(point, lift: lift)
        }
        // Its own transaction, and the only one here. See `syncMenu`.
        syncMenu()
    }

    /// Which column row a touch at `point` chose, or nil when there was no
    /// open column under it.
    ///
    /// **One function for the tap and the drag both**, which is the whole of
    /// the second half of this change. The tap has always been measured this
    /// way; the drag used to be measured off its own travel inside
    /// `barRelease`, and two answers to "which row is that" is how a menu
    /// comes to disagree with itself by exactly one row.
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
    private func columnRow(at point: CGPoint, lift: CGFloat) -> Int? {
        guard let above = columnAbove(point, lift: lift) else { return nil }
        return ShellGesture.columnRow(above: above, tabCount: tabCount)
    }

    /// How far `point` is above the bar row's top edge, or nil while there is
    /// no column for it to be above.
    ///
    /// The guard is the same one a release is gated on —
    /// `ShellGesture.columnHeight` is non-zero exactly when the column is
    /// pinned or the lift has passed `openMin` — so the highlight and the
    /// landing appear and disappear together rather than at two thresholds.
    ///
    /// `lift` is passed rather than read off the state it was just written to.
    /// `onChanged` writes `lift` a line above the call, and whether a `@State`
    /// getter returns a value set in the same closure is not something worth
    /// depending on for the frame a column opens in.
    private func columnAbove(_ point: CGPoint, lift: CGFloat) -> CGFloat? {
        guard barBottom > 0,
            ShellGesture.columnHeight(up: lift, tabCount: tabCount, pinned: columnPinned) > 0
        else { return nil }
        return aboveBarTopEdge(point.y)
    }

    /// The same number `columnAbove` guards on, without the guard.
    ///
    /// Pulled out so the touch-down anchor `begin` hands `ShellBarDrag` — see
    /// `ShellBarDrag.startAbove` — is measured exactly the same way the
    /// column's own row selection is, rather than a second formula that
    /// could drift from it. It is safe to ask before any column is showing:
    /// a touch anywhere on the bar itself answers a small negative number
    /// (the bar's own row is `ShellMetrics.barRow` tall), never a number a
    /// real column row could be confused with, and `pageRise`'s own
    /// `max(0, …)` treats every negative alike regardless.
    private func aboveBarTopEdge(_ y: CGFloat) -> CGFloat {
        (barBottom - ShellMetrics.barRow) - y
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
    /// `minimumDistance: 0` still, and zero rather than the axis rule's six so
    /// `decideAxis` stays the only thing deciding what a drag means.
    func contentGesture(page: CGFloat) -> some Gesture {
        // `.global` for the same reason the bar's is, kept the same here so
        // the two gestures cannot come to measure different things.
        DragGesture(minimumDistance: 0, coordinateSpace: .global)
            .onChanged { value in
                begin(
                    on: .content, from: value.startLocation.x - pageFrame.minX,
                    at: value.startLocation)
                noteMovement(value)
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
                // The peak, kept because the defect is a transient: a shell
                // that moves and then puts itself back reads as a shell that
                // held still at every moment a test can sample. See
                // `ShellRootView.strayed`.
                strayed = max(strayed, abs(trackX))
            }
            .onEnded { value in
                let decided = axis
                let travelled = value.translation.width
                // **A drag the pane used ANY of carries no momentum to the
                // shell**, and this is the one line of this change that was
                // found by a test rather than reasoned out.
                //
                // A drag the pane absorbed entirely arrives as zero distance,
                // which `contentRelease` already reads as a spring-back — but
                // a velocity is not zeroed by a subtraction. So the first
                // version of this zeroed the velocity only while the pane was
                // STILL absorbing, and let it through once the hunk hit its
                // edge. `testACodeLineAtItsEndHandsThePageTurnBack` went red:
                // *"reading the line to its end turned the page on the way"*.
                // Sixty points of drag, twenty-five of them spent scrolling a
                // code line to its end and thirty-five handed over — and a
                // release velocity from the whole sixty projected the
                // thirty-five past four hundred.
                //
                // Which is the jump `carriedX` exists to prevent, arriving
                // through the other axis of the same gesture. The handoff is
                // there so that a page turn "begins from zero rather than
                // jumping to wherever the finger had got to"; momentum carried
                // across it is that jump, restated. So the rule is the whole
                // gesture rather than the frame: `handoff` is non-zero exactly
                // when the pane took some of this drag, and a drag the pane
                // was part of has to earn its seventy points on travel alone.
                //
                // A flick that runs off the end of a line still turns the page
                // — it takes the next gesture, which is what the second half
                // of that same test does and what a nested scroller at its
                // edge does everywhere else on this platform.
                let panesOwn = handoff != 0 || dragClaim.room.absorbs(dx: travelled) > 0.5
                // The same subtraction the release is measured against.
                let dx = carriedX(travelled)
                rest()
                apply(
                    fleet.contentRelease(
                        axis: decided, dx: dx, at: position,
                        dxVelocity: panesOwn ? 0 : releaseVelocity(value).width),
                    dx: dx, page: page, wasOpen: false)
                syncMenu()
            }
    }

    /// The first `onChanged` of a gesture, and only the first.
    ///
    /// `startAbove` is `.bar`-only — the content has no column to be above —
    /// and defaults to zero, which is what `contentGesture`'s call gets by
    /// never mentioning it. It seeds `barDrag` fresh, in the same breath as
    /// every other per-gesture reset here, with the one fact about the touch
    /// `ShellBarDrag` could not otherwise have: where it landed. See
    /// `ShellBarDrag.startAbove`'s header.
    private func begin(
        on which: ShellTrack, from originX: CGFloat, at point: CGPoint = .zero,
        startAbove: CGFloat = 0
    ) {
        guard !gestureActive else { return }
        gestureActive = true
        // **What is under this finger, asked now rather than waited for.**
        //
        // Here rather than at the end of the last gesture for the reason it
        // always was: `minimumDistance: 0` means this runs the moment a finger
        // lands, before anything under it can have scrolled, which is the only
        // instant at which the answer is a fact about the LAYOUT rather than
        // about a drag already in progress.
        //
        // It is a lookup and no longer a clear, and that is the fix for the
        // owner's report. A hunk used to speak only once its own scroll view
        // had begun, which costs UIKit its usual slop plus a frame, and the
        // shell drew a page turn through every point of it and then took it
        // back — 36 points out and back, measured, on every horizontal drag
        // over code. `ShellScroller` carries the argument; what changed here
        // is only which of the two parties asks first.
        //
        // `.bar` finds nothing, because it is asked about a point on the bar
        // and nothing on the bar scrolls sideways — but it is spelled out
        // rather than left to the geometry: the bar's own gesture has no
        // handoff at all, and a claim leaking into it would be a worktree
        // swipe silently absorbed by a diff two panes away.
        dragClaim.room = which == .content ? dragClaim.roomUnder(point) : .none
        handoff = 0
        // The two probe channels, cleared with everything else a new finger
        // invalidates. See `ShellRootView.strayed` and `.lockedOn` — both are
        // read by the UI suite after the finger has come up, so a gesture owns
        // them from its own touch-down until the next one.
        strayed = 0
        lockedOn = .zero
        // No movement yet, so nothing recent enough to be momentum. A gesture
        // that ends here without ever moving is a tap, and a tap throws
        // nothing.
        lastMoved = nil
        track = which
        wasOpen = columnPinned
        liftOrigin = originX
        if which == .bar { barDrag = ShellBarDrag(startAbove: startAbove) }
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
            // after all. This used to be the ordinary case and is now the
            // exception: the claim is seeded at touch-down — see `begin` —
            // so a hunk that was laid out before the finger landed is
            // answering from the first frame and there is nothing to put
            // back. What is left for this to catch is a scroller that gained
            // room DURING the drag, which is a real thing a diff does when a
            // hunk's widest row finishes measuring. Put back rather than left
            // standing either way: a page parked two points off center for
            // the rest of a read is a page nobody asked to move.
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
            if trackX != 0 {
                withAnimation(Self.tracking) {
                    trackX = 0
                }
            }
            return 0
        }
        return dx - handoff
    }

    /// Remember when the finger last actually moved.
    ///
    /// Half a point of slop, because the question is whether the finger is
    /// travelling and not whether the digitizer jittered. See
    /// `ShellRootView.lastMoved`, which is where the measurements that make
    /// this necessary are written down.
    private func noteMovement(_ value: DragGesture.Value) {
        guard let last = lastMoved else {
            lastMoved = (at: value.translation, time: value.time)
            return
        }
        guard abs(value.translation.width - last.at.width) > 0.5
            || abs(value.translation.height - last.at.height) > 0.5
        else { return }
        lastMoved = (at: value.translation, time: value.time)
    }

    /// The velocity a release is actually entitled to.
    ///
    /// `value.velocity` when the finger was still moving, and flatly zero when
    /// it had stopped — see `ShellRootView.lastMoved` for why the second half
    /// cannot be left to the estimator. Zero rather than a decay, because
    /// there is nothing to decay: a finger that has been parked for four
    /// frames is not going anywhere, and a projection is a claim about where
    /// it was going.
    private func releaseVelocity(_ value: DragGesture.Value) -> CGSize {
        guard let last = lastMoved,
            value.time.timeIntervalSince(last.time) <= Self.stillFor
        else { return .zero }
        return value.velocity
    }

    /// Decide the axis, once, for the CONTENT.
    ///
    /// The guard is the whole rule: once `axis` is non-nil nothing asks again
    /// for the rest of this gesture. Vertical on the content is the
    /// terminal's scrollback, so an axis that could be revisited per frame is
    /// the shell reaching into a gesture the pane owns — the failure class of
    /// `b192f17`, which shipped and was reported off a real phone. The bar
    /// asks `ShellBarDrag` instead, once per frame; `ShellGesture.lean` is
    /// where the difference between the two surfaces is argued.
    ///
    /// **Because the answer is final, the question is `contentAxis` and not
    /// `axis`.** One sample decides where a six-hundred-point scroll ends up,
    /// and `lockedOn` says which sample: about seven points across against six
    /// up, measured on a simulator. A bare magnitude comparison at that scale
    /// is deciding on the roll a thumb makes as it lands.
    private func decideAxis(dx: CGFloat, up: CGFloat) {
        guard axis == nil else { return }
        axis = ShellGesture.contentAxis(dx: dx, up: up)
        // The sample the decision was made from, kept for the probe. Stored
        // up-positive, the way `contentAxis` is asked, so `locky` and the rule
        // agree about which way is up.
        if axis != nil { lockedOn = CGSize(width: dx, height: up) }
    }

    /// The half of a handover that is a transaction rather than a number.
    ///
    /// `ShellBarDrag` has already charged the claimed channel so that it
    /// starts from where the finger is now — the rule `carriedX` states for a
    /// pane's edge, restated for an axis. What is left is the abandoned
    /// channel, and putting it back is not the finger's position any more: it
    /// is a page turn that is not going to happen, or a menu that is not
    /// being chosen from. Easing an apology is the one thing in this file
    /// `Self.tracking` is still for — the argument is in `carriedX`, and it
    /// is the same argument. Snapped, a redirection reads as a glitch; eased,
    /// it reads as the shell letting go of one answer and offering the other,
    /// which is what the talk means by hinting in the direction of the
    /// gesture.
    ///
    /// **It runs on the frame the gesture changed its mind and never again**,
    /// which is why the arms of `onChanged` do not zero each other's channel:
    /// a raw write of the same zero on the next frame would snap this ease
    /// flat.
    ///
    /// There is no arm for a page in your hand, because there cannot be one:
    /// `ShellGesture.lean` answers `.vertical` unconditionally once
    /// `ShellBarDrag.holdingPage` is latched, so a held page is never handed
    /// anywhere. That is also why nothing here puts back `carryX`: it is
    /// zero for the whole stretch in which a handover can happen at all.
    private func handOver(to claimed: ShellAxis) {
        switch claimed {
        case .horizontal:
            fingerAbove = nil
            // `pageAbove` alongside `lift`: it is the same put-back, for the
            // same page. In practice this is never a visible ease — a
            // handover to horizontal can only happen while `holdingPage` is
            // still false, which is exactly the stretch in which `pageAbove`
            // has never exceeded the column's own run — but it is put back
            // for the reason `lift` is: a value left standing after the
            // channel that drew it let go of the gesture is a value that no
            // longer describes anything.
            withAnimation(Self.tracking) {
                lift = 0
                pageAbove = 0
            }
        case .vertical:
            withAnimation(Self.tracking) {
                trackX = 0
            }
        }
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
    /// The page falls back the same way and by the same spring: `lift` is
    /// what holds it up, so letting go of the bar drops the screen back onto
    /// the display.
    ///
    /// **It does NOT inherit the finger's velocity, and that is a real gap
    /// rather than a decision.** This used to say SwiftUI handed the in-flight
    /// `interactiveSpring` over to this one — which was true while `lift` was
    /// written inside a tracking spring, and stopped being true the moment it
    /// was written raw so the page could follow the finger one point per
    /// point. There is no in-flight animation to hand anything over now, so a
    /// page thrown upward and released still starts its fall from a
    /// standstill. The decisions the throw feeds are all fixed —
    /// `barRelease` reads `value.velocity` — but the MOTION of the fall is
    /// not, and the fix is not a parameter on this line: `Animation.spring`
    /// takes no initial velocity, and the only SwiftUI animation that does,
    /// `interpolatingSpring`, is ADDITIVE — with `minimumDistance: 0` a
    /// finger landing mid-settle writes `lift` raw and would draw it plus
    /// whatever the interrupted spring had left. That is a worse defect than
    /// the one it fixes, so the lift wants what the terminal already has:
    /// its own stepped physics rather than a SwiftUI spring.
    ///
    /// `trackX` is deliberately NOT reset here, and neither is `carryX`. They
    /// are the shell's translation rather than facts about the gesture, every
    /// arm of `apply` resolves both exactly once, and zeroing them here would
    /// make a commit animate from the center as a full-bleed page — the page
    /// jumping back and flattening before it goes.
    private func rest() {
        gestureActive = false
        axis = nil
        wasOpen = false
        // There is no finger, so there is no row under one. Cleared here for
        // the reason everything else in this function is: a highlight left
        // standing after the gesture that placed it would be a pinned column
        // pointing at a row nobody is touching.
        fingerAbove = nil
        // And no axis, no handover, and nothing in anybody's hand. Here
        // rather than in `begin` because `menuShouldShow` reads it BETWEEN
        // gestures: a `spentLift` left standing over a tap-pinned column is a
        // menu suppressed by a redirection that finished a minute ago. Both
        // `onEnded`s spend what they need of it before calling this. `begin`
        // ALSO resets it, with the next gesture's own touch-down anchor —
        // the two are not redundant, they cover the two different windows.
        barDrag = ShellBarDrag()
        // `settled`, not `settle`: the thumb went UP and the page falls DOWN,
        // so there is no momentum here to reward. See `ShellRootView.settled`,
        // which carries the trace — the bounce was thirteen frames of a page
        // that had already finished.
        //
        // `pageAbove` falls the same way and on the same spring as `lift`:
        // it is what `ShellPageLayer.pageRise` now reads to draw the fall,
        // so leaving it standing here would freeze the page wherever the
        // gesture last left it while `lift` alone sprang back to zero.
        withAnimation(Self.settled) {
            lift = 0
            pageAbove = 0
        }
    }

    /// A pinned column's row, tapped on the row itself.
    ///
    /// The same landing a drag's release makes, through the same arm of the
    /// same function — `.land` is where the tab changes, where the column
    /// furls and where the animation that does both is chosen, and a second
    /// copy of any of that is a second thing to keep in step. What differs is
    /// only where the row came from: a button knows its own index, and a
    /// release has to be told one by `ShellGesture.columnRow`.
    ///
    /// `dx` and `page` are the release's, and `.land` reads neither; `wasOpen`
    /// belongs to `.toggleColumn`. Passed as the honest values for a tap on a
    /// column that is open: no sideways travel, and the column was open.
    ///
    /// `syncMenu` afterwards, exactly as `onEnded` does it, because the furl
    /// is a transaction of its own — see that function's header.
    func chooseRow(_ index: Int) {
        apply(.land(tab: index), dx: 0, page: 0, wasOpen: true)
        syncMenu()
    }

    private func apply(_ release: ShellRelease, dx: CGFloat, page: CGFloat, wasOpen: Bool) {
        switch release {
        case .commit(let step):
            commit(step, dx: dx, page: page)
        case .springBack, .abandon, .openOverview, .carry:
            // A shell over one worktree has no overview to reach and no
            // neighbor to carry a page to, so a lift the model says would have
            // gone to either is put back down, as one that fell short is.
            withAnimation(Self.settle) { flatten() }
        case .land(let tab):
            // `settled`, for the reason a tapped menu is: choosing a row has
            // no momentum toward the row — the finger is resting on it. See
            // `ShellRootView.settled`, including the trace showing that this
            // animation currently carries nothing at all.
            withAnimation(Self.settled) {
                position.tab = tab
                // Landing on a row is choosing from the column, so the column
                // has done its job. It furls whether it was pinned or dragged
                // — a tap-opened column that stayed open after a choice would
                // leave the chosen pane behind a list of its siblings.
                columnPinned = false
                // And the row's own report goes with it, at every one of the
                // five places that clear `columnPinned`.
                //
                // `ShellColumn.onTouch` is `ShellRowPress` reporting
                // `ButtonStyle.isPressed`, and what is not safe to depend on is
                // its TRAILING edge: this line furls the column, the rows stop
                // being composed, and a style whose body has gone never
                // delivers the `false`. A drag release reaches this arm with no
                // button pressed at all, so whatever the last press left is
                // simply still standing. Either way a stale row would be lit in
                // the column the NEXT tap opens.
                touchedRow = nil
                flatten()
            }
        case .toggleColumn:
            withAnimation(Self.settle) {
                columnPinned = !wasOpen
                // See `.land` above: a press whose trailing edge the column
                // outlived must not outlive the column itself.
                touchedRow = nil
                flatten()
            }
        }
    }

    /// The page's position, back to a full-bleed page on the display.
    ///
    /// Called INSIDE the animation of whichever arm of `apply` ran, never
    /// before the release has decided — see `rest()`. One function rather than
    /// four copies because the failure mode of a copy is silent: a new arm
    /// that forgets `carryX` leaves the page held off to one side.
    private func flatten() {
        trackX = 0
        carryX = 0
    }

    /// Animate to the neighbor, then re-seat on it without animating.
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
            carryX = 0
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

    // MARK: - The way back out, under a thumb

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
    /// A silent re-seat: no animation, because there is no gesture and nothing
    /// on screen moved toward it. That is the pane host's retarget, arrived at
    /// from the shell's side, and it is what keeps every mounted pane exactly
    /// where it was.
    func honorRequest() {
        guard let id = request else { return }
        request = nil
        guard let target = fleet.position(ofTab: id) else { return }
        if target != position {
            var silent = Transaction()
            silent.disablesAnimations = true
            withTransaction(silent) {
                position = target
                // The column lists the worktree it belongs to. A link that
                // crosses worktrees would otherwise leave it open over a list
                // of somebody else's tabs; one that does not still moves the
                // tab out from under the highlighted row.
                columnPinned = false
                touchedRow = nil
            }
        }
    }
}
