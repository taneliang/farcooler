package com.farcooler.ui

import androidx.compose.animation.core.animate
import androidx.compose.animation.core.spring
import androidx.compose.foundation.gestures.FlingBehavior
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.ScrollScope
import androidx.compose.foundation.gestures.rememberScrollableState
import androidx.compose.foundation.gestures.scrollable
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.onSizeChanged
import com.farcooler.model.ShellAxis
import com.farcooler.model.ShellDirection
import com.farcooler.model.ShellFleet
import com.farcooler.model.ShellGesture
import com.farcooler.model.ShellMetrics
import com.farcooler.model.ShellPosition
import com.farcooler.model.ShellRelease
import com.farcooler.model.ShellStep
import com.farcooler.model.ShellTab
import com.farcooler.model.ShellTrack
import com.farcooler.model.ShellTrackGeometry
import com.farcooler.model.ShellWorkspace
import com.farcooler.model.contentRelease
import com.farcooler.model.Workspace

/**
 * The shell's content track: the pane you are on, with a real one either side.
 *
 * Swipe sideways and the neighbouring pane comes in under your thumb. It is a
 * genuinely mounted, genuinely composed pane — not a placeholder that becomes a
 * terminal once you let go — which is the whole reason `PaneDeck.MOUNT_LIMIT`
 * is five rather than three.
 *
 * ## Never a pager. Never a lazy anything.
 *
 * `HorizontalPager` is a `LazyLayout`: `beyondViewportPageCount` defaults to 0
 * and off-screen pages are DISPOSED. Disposing a page here disposes the
 * `AndroidView` under a terminal, which is a native alacritty grid bounded by
 * `SCROLLBACK_LINES = 10_000` plus a dedicated emulator thread — so the
 * convenience API for exactly this interaction is the one thing that must not be
 * used for it. The same goes for `LazyRow`.
 *
 * So the panes stay where they already were: a plain `Box` in `WorkspaceScreen`,
 * one `key(pane.id)` per mounted pane, all of them composed all of the time.
 * This file adds two modifiers over that and destroys nothing. At a limit of
 * five the cost of getting it wrong went up, not down.
 *
 * ## The page turn yields to content that can still scroll sideways
 *
 * A wide diff has to scroll under your thumb and hand the page turn over only at
 * its own edge. iOS does this by having panes report how much horizontal room
 * they have left and subtracting it in the shell.
 *
 * **Android has the mechanism already and it is better**: nested scrolling. The
 * track is a `Modifier.scrollable` wrapping the panes, so a child
 * `horizontalScroll` — every row of a patch in `AgentRows` is one — receives the
 * gesture first, consumes what it can use, and dispatches only the leftover up
 * to this connection. A diff in the middle of its own range consumes everything
 * and the page does not move; at its edge the remainder arrives here and the
 * page turns. No branch anywhere knows what a diff is, and nothing had to
 * measure anything.
 *
 * It also disposes of the axis problem rather than re-solving it. iOS's
 * hand-rolled lock shipped `abs(dx) > dy` with `dy` up-positive, so **every
 * downward drag classified as horizontal** and scrolling a transcript turned the
 * page; a real phone found it. `scrollable(Horizontal)` will not start until a
 * gesture crosses the platform's touch slop HORIZONTALLY, and a vertical drag
 * belongs to whatever vertical scroller is under it. `ShellGesture.axis` is
 * still the right thing for the bar, where the vertical half of the gesture is
 * the column unfurling and there is no scroller to hand it to.
 *
 * ## What is still decided by the shared model
 *
 * Everything with meaning: [ShellGesture.commits] and `direction` for whether
 * and where, [ShellFleet.contentRelease] and `step` for what a release resolves
 * to, [ShellGesture.translation] for resistance at the ends, and
 * [ShellTrackGeometry] for where panes sit and what a commit does to them. The
 * platform supplies the input plumbing; the model supplies the answers, and it
 * is the same model iOS and the Mac read.
 */
class PaneTrackState internal constructor() {

    /** How far the track has been dragged, in pixels. Negative is leftward. */
    var offset by mutableFloatStateOf(0f)
        private set

    /** One page, measured. See [ShellMetrics] for why there is no constant. */
    var pageWidth by mutableFloatStateOf(0f)
        internal set

    /** Whether anything is off its resting place, so neighbours need drawing. */
    val moving: Boolean get() = offset != 0f

    /**
     * How far the finger has actually gone, before resistance.
     *
     * Kept apart from [offset] so that [ShellGesture.translation] is applied to
     * the whole travel rather than to each delta as it arrives. Scaling deltas
     * one at a time gives the same answer only while the sign never changes; a
     * drag that goes past the end, comes back and goes out again would
     * accumulate a different number every time, which is a drift nobody would
     * ever reproduce deliberately.
     */
    private var travel = 0f

    /**
     * Take a horizontal delta the panes did not want.
     *
     * Called from the nested-scroll connection, so by construction this only
     * ever sees what a child `horizontalScroll` could not use — a wide diff in
     * the middle of its range consumes everything and nothing reaches here.
     *
     * Returns the whole delta as consumed. There is nothing above the track that
     * wants horizontal scroll, and reporting less would let an ancestor start
     * moving as well.
     */
    internal fun drag(delta: Float, fleet: ShellFleet, position: ShellPosition): Float {
        if (pageWidth <= 0f) return 0f
        travel += delta
        val direction = ShellGesture.direction(travel)
        val banding =
            direction != null && fleet.rubberBands(position, direction, ShellTrack.CONTENT)
        offset = ShellGesture.translation(travel, banding)
        return delta
    }

    /**
     * Decide what the release meant, and land it.
     *
     * **The commit does not animate to the neighbour and then reset.** It
     * re-parameterises in place — [ShellTrackGeometry.residual] against
     * [onCommit], inside one snapshot so no frame can land between them — and
     * then settles the remainder to zero. Every pane is drawn at exactly the
     * pixel it was already at across the swap, so there is no jump to hide and
     * no completion callback to get right. `ShellTest.committingMovesNothing`
     * is that claim, asserted for every slot.
     */
    internal suspend fun settle(
        fleet: ShellFleet,
        position: ShellPosition,
        velocity: Float,
        onCommit: (ShellStep) -> Unit,
    ) {
        travel = 0f
        val release = fleet.contentRelease(ShellAxis.HORIZONTAL, offset, position, velocity)
        // Where the throw was HEADED, which is what decided the release, so
        // the direction it commits in is the same number rather than a second
        // reading of the same gesture. A drag one way flicked back the other
        // way at the last moment turns the page the way it is going.
        val direction = ShellGesture.direction(ShellGesture.projected(offset, velocity))
        if (release is ShellRelease.Commit && direction != null) {
            Snapshot.withMutableSnapshot {
                offset = ShellTrackGeometry.residual(offset, direction, pageWidth)
                onCommit(release.step)
            }
        }
        val from = offset
        if (from == 0f) return
        animate(
            initialValue = from,
            targetValue = 0f,
            // Interruptible by construction: a new gesture starts a new
            // `scrollable` drag, which cancels the scope this is running in.
            animationSpec = spring(dampingRatio = 0.82f, stiffness = 380f),
        ) { value, _ -> offset = value }
    }
}

/**
 * The whole fleet as the content track walks it — which today is one workspace.
 *
 * **The tab ids ARE [Pane] ids**, which is the join that lets a `ShellFleet`
 * describe a deck without either type knowing about the other.
 *
 * **One workspace, and that is a limitation with a name.** `model/Shell.kt`'s
 * `contentStep` walks one flat sequence across every workspace, and the mount
 * limit was raised to five specifically so a neighbour in ANOTHER workspace
 * could be held. It cannot be yet: [Pane.Changes] is a `data object`, so every
 * workspace's diff shares one identity, and `Pane.id` is what `key()` and the
 * `SaveableStateHolder` bucket on — two workspaces in one deck would hand one
 * worktree's diff the other's saved state. Giving `Changes` a workspace also
 * changes a string that is PERSISTED in the focus map, so it needs the same kind
 * of migration `Pane.parse`'s third arm already carries.
 *
 * That is a separate piece of work and it is deliberately not smuggled in here.
 * What matters is that nothing in the track knows: it asks a `ShellFleet` for
 * its neighbours, and the day the fleet handed to it spans workspaces, the
 * cross-workspace case starts working with no change to any of this.
 *
 * Order matches `TerminalTabStrip`: the diff leads, then the panes in fleet
 * order. A track whose sequence disagreed with the strip's would be two
 * different answers to "what is next to this".
 */
fun trackFleet(workspace: Workspace?, workspaceId: String, hostId: String): ShellFleet {
    val terminals = workspace?.terminals.orEmpty().filterNot { it.isChangesPane }
    val tabs = buildList {
        add(ShellTab(id = Pane.CHANGES_ID, title = "Diff", mark = null))
        terminals.forEach {
            add(ShellTab(id = Pane.Terminal(it.id).id, title = it.label, mark = null))
        }
    }
    return ShellFleet(
        listOf(
            ShellWorkspace(
                id = workspaceId,
                name = workspace?.short.orEmpty(),
                tabs = tabs,
                runnerId = hostId,
            )
        )
    )
}

/**
 * Which slot a pane occupies on the track, or null when it is off it.
 *
 * Null is the ordinary state of the panes the deck holds that are not your
 * neighbours: composed, alive, keeping their scrollback and their half-typed
 * message, and simply not drawn.
 */
fun trackSlot(fleet: ShellFleet, position: ShellPosition, pane: Pane): Int? {
    if (fleet.tab(position)?.id == pane.id) return 0
    val previous = fleet.step(position, ShellDirection.PREVIOUS, ShellTrack.CONTENT)
    if (previous != null && fleet.tab(previous.position)?.id == pane.id) return -1
    val next = fleet.step(position, ShellDirection.NEXT, ShellTrack.CONTENT)
    if (next != null && fleet.tab(next.position)?.id == pane.id) return 1
    return null
}

/** The track's state, held across recompositions. */
@Composable
fun rememberPaneTrack(): PaneTrackState = remember { PaneTrackState() }

/**
 * The container's half: measure a page, and take the horizontal deltas the panes
 * did not want.
 *
 * @param onCommit called once a page turn is decided, with the offset already
 *   re-parameterised so that nothing moves. Make [ShellStep.position] current;
 *   the settle is this file's job.
 */
@Composable
fun Modifier.paneTrack(
    state: PaneTrackState,
    fleet: ShellFleet,
    position: ShellPosition,
    onCommit: (ShellStep) -> Unit,
): Modifier {
    // Read through holders so the scroll and fling objects — remembered once —
    // always see the frame they are running in. Capturing `fleet` would let a
    // swipe resolve against the shape the workspace had when the screen opened,
    // which is a stale answer on any runner that is still starting panes.
    val fleetNow by rememberUpdatedState(fleet)
    val positionNow by rememberUpdatedState(position)
    val commitNow by rememberUpdatedState(onCommit)

    val scrollable = rememberScrollableState { delta -> state.drag(delta, fleetNow, positionNow) }
    val fling = remember(state) {
        object : FlingBehavior {
            override suspend fun ScrollScope.performFling(initialVelocity: Float): Float {
                // **The velocity is spent rather than dropped**, which it used
                // to be. The note that stood here said distance decides and not
                // velocity, "the model's rule and the one the other two
                // platforms follow" — and it was an accurate description of a
                // defect on all three. A release that reads only where the
                // finger stopped is the talk's own counter-example: forty units
                // thrown hard and forty placed deliberately mean opposite
                // things, and the shell answered both with a spring-back.
                //
                // `ShellGesture.projected` inside `contentRelease` is where it
                // is spent. Nothing is consumed here — the whole fling is the
                // settle, and reporting any of it back would let an ancestor
                // scroll on what this one used.
                state.settle(fleetNow, positionNow, initialVelocity, commitNow)
                return 0f
            }
        }
    }
    return this
        .onSizeChanged { state.pageWidth = it.width.toFloat() }
        .scrollable(state = scrollable, orientation = Orientation.Horizontal, flingBehavior = fling)
}

/**
 * A pane's half: where it sits, and whether it is drawn at all.
 *
 * `graphicsLayer` with a lambda, so a moving track re-reads [PaneTrackState] in
 * the draw phase rather than recomposing every pane on every frame of a drag.
 * Recomposing a terminal sixty times a second would be a different kind of
 * expensive from destroying one, and it is just as avoidable.
 */
fun Modifier.trackedPane(state: PaneTrackState, slot: Int?, showing: Boolean): Modifier =
    this.graphicsLayer {
        if (slot == null) {
            alpha = 0f
            return@graphicsLayer
        }
        translationX = ShellTrackGeometry.x(slot, state.pageWidth, state.offset)
        // The pane you are on is never dimmed. A neighbour is drawn only while
        // the track is off its resting place — at rest it sits exactly one page
        // off-screen, and painting it there would be a full-screen layer
        // composited every frame for nothing.
        alpha = when {
            showing -> 1f
            state.moving -> ShellTrackGeometry.NEIGHBOUR_ALPHA
            else -> 0f
        }
    }
