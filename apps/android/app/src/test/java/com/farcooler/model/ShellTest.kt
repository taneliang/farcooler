package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.sin
import org.junit.Test

/**
 * The shell's arithmetic, pinned.
 *
 * **Three of these tests exist because iOS shipped without them.** `b192f17`
 * fixed three gesture bugs found after ten minutes on a real phone, and all
 * three were pure functions of numbers: the axis sign, the column row's
 * inversion, and a pull-down gated on the wrong end of a gesture. Each has a
 * test here named for the symptom rather than the function, so that a future
 * edit that reintroduces one fails with the report a person would file.
 *
 * The rest is the ordinary surface: stepping, thresholds, releases and the
 * overview's order.
 */
class ShellTest {

    // MARK: - Fixtures

    private fun tab(id: String, wants: Boolean = false, stale: Boolean = false) = ShellTab(
        id = id,
        title = id,
        mark = GlanceMark(
            if (wants) GlanceMark.Attention.NEEDS_YOU else GlanceMark.Attention.QUIET,
            GlanceMark.Core.AT_A_PROMPT,
            if (stale) GlanceMark.Link.BROKEN else GlanceMark.Link.LIVE,
        ),
        wantsAttention = wants,
    )

    private fun diffTab(id: String, unread: Boolean) = ShellTab(
        id = id,
        title = "Diff",
        mark = GlanceMark(
            if (unread) GlanceMark.Attention.TO_REVIEW else GlanceMark.Attention.QUIET,
            GlanceMark.Core.AT_A_PROMPT,
        ),
    )

    private fun workspace(
        id: String,
        tabs: List<ShellTab>,
        runner: String = "laptop",
        resume: Int? = null,
    ) = ShellWorkspace(id = id, name = id, tabs = tabs, runnerId = runner, resume = resume)

    /** Two workspaces of two tabs, on one runner. Four tabs in the flat sequence. */
    private val simple = ShellFleet(
        listOf(
            workspace("alpha", listOf(tab("a0"), tab("a1"))),
            workspace("beta", listOf(tab("b0"), tab("b1"))),
        )
    )

    // MARK: - The axis, and the bug that shipped

    /**
     * **The bug, as reported: vertical scrolling turned the page.**
     *
     * `ShellGesture.axis` read `abs(dx) > dy` with `dy` up-positive, carried
     * verbatim from the web prototype. A downward drag has NEGATIVE `dy`, so it
     * lost to any horizontal component at all and every downward drag
     * classified as horizontal.
     *
     * The numbers are the ones off the phone: 79dp across while travelling
     * 511dp down, about 9° off vertical — and 79 is past
     * [ShellMetrics.PAGE_COMMIT], so it did not merely misclassify, it committed.
     */
    @Test
    fun `a thumb dragging down the screen is vertical, not horizontal`() {
        assertEquals(ShellAxis.VERTICAL, ShellGesture.axis(dx = 79f, up = -511f))
        assertEquals(ShellAxis.VERTICAL, ShellGesture.axis(dx = -79f, up = -511f))
    }

    /** The same claim from the other side: an upward drag was never broken. */
    @Test
    fun `a thumb dragging up the screen is vertical too`() {
        assertEquals(ShellAxis.VERTICAL, ShellGesture.axis(dx = 79f, up = 511f))
    }

    @Test
    fun `a mostly sideways drag is horizontal in both directions`() {
        assertEquals(ShellAxis.HORIZONTAL, ShellGesture.axis(dx = 120f, up = 20f))
        assertEquals(ShellAxis.HORIZONTAL, ShellGesture.axis(dx = -120f, up = -20f))
    }

    /**
     * The lock: nothing is decided inside 6dp, in any direction. A gesture that
     * classified on the first pixel would classify on noise.
     */
    @Test
    fun `the axis does not commit inside the lock`() {
        assertNull(ShellGesture.axis(dx = 4f, up = 4f))
        assertNull(ShellGesture.axis(dx = -6f, up = 0f))
        assertNull(ShellGesture.axis(dx = 0f, up = -6f))
        assertTrue(ShellGesture.axis(dx = 6.5f, up = 0f) != null)
    }

    /**
     * A perfect diagonal resolves vertical rather than throwing, and it must
     * resolve the SAME way every frame — the tie-break is `>` on the horizontal
     * side, so equal magnitudes are vertical.
     */
    @Test
    fun `a perfect diagonal is vertical and is stable`() {
        assertEquals(ShellAxis.VERTICAL, ShellGesture.axis(dx = 40f, up = 40f))
        assertEquals(ShellAxis.VERTICAL, ShellGesture.axis(dx = -40f, up = -40f))
    }

    // MARK: - Redirection

    /** One frame of a bar drag, folded over a whole path the way a frame loop does. */
    private fun drive(
        path: List<Pair<Float, Float>>,
        tabCount: Int,
    ): Triple<ShellBarDrag, ShellBarDrag.Frame, Int> {
        val drag = ShellBarDrag()
        var frame = ShellBarDrag.Frame(null, 0f, 0f, null)
        var flips = 0
        for ((dx, up) in path) {
            frame = drag.moved(dx, up, tabCount)
            if (frame.claimed != null) flips++
        }
        return Triple(drag, frame, flips)
    }

    /** A straight run of samples, the way a finger at a constant speed arrives. */
    private fun leg(
        from: Pair<Float, Float>,
        to: Pair<Float, Float>,
        steps: Int = 24,
    ): List<Pair<Float, Float>> =
        (1..steps).map { i ->
            val t = i.toFloat() / steps
            Pair(from.first + (to.first - from.first) * t, from.second + (to.second - from.second) * t)
        }

    /**
     * **A drag that goes sideways and then turns upward opens the column**, and
     * the same ninety points that do not turn are the workspace crossing they
     * always were.
     *
     * The owner's ask: *"the user can start swiping horizontally, then decide
     * they want to swipe vertically instead."* Nothing asserted it before,
     * because nothing could: the axis was decided on the first six points and
     * never revisited.
     */
    @Test
    fun `a drag that turns upward out of a sideways swipe reaches the column`() {
        val turned = drive(leg(0f to 0f, -90f to 0f) + leg(-90f to 0f, -90f to 200f), 3)
        assertEquals(ShellAxis.VERTICAL, turned.second.axis)
        assertEquals(1, turned.third)
        // The sideways it spent on the page turn it abandoned, so the next
        // thing to read `dx` starts from nothing instead of jumping 90dp.
        assertEquals(-90f, turned.first.spentSideways, 0.01f)
        assertEquals(0f, turned.second.sideways, 0.01f)

        val straight = drive(leg(0f to 0f, -90f to 0f), 3)
        assertEquals(ShellAxis.HORIZONTAL, straight.second.axis)
        assertEquals(0, straight.third)
    }

    /**
     * **And the mirror: a lift that turns sideways crosses the workspace.**
     *
     * The half that looks like it already worked and did not. A lifted page
     * could go sideways, but only past the last row where the two axes stop
     * competing; inside the column sideways meant nothing at all however
     * decisive it was.
     */
    @Test
    fun `a lift that turns sideways out of an open column crosses the workspace`() {
        val turned = drive(leg(0f to 0f, 0f to 100f) + leg(0f to 100f, -220f to 100f), 3)
        assertEquals(ShellAxis.HORIZONTAL, turned.second.axis)
        assertEquals(1, turned.third)
        assertTrue(turned.first.spentSideways < -139f && turned.first.spentSideways > -150f)
        assertEquals(
            "a charge on the lift belongs to whoever is holding the vertical",
            0f,
            turned.first.spentLift,
            0.01f,
        )

        // A hundred points of wander off the same lift is short of taking the
        // gesture, and the column keeps it.
        val wandered = drive(leg(0f to 0f, 0f to 100f) + leg(0f to 100f, -100f to 100f), 3)
        assertEquals(ShellAxis.VERTICAL, wandered.second.axis)
        assertEquals(0, wandered.third)
    }

    /**
     * **A straight drag means exactly what it meant before any of this**, at
     * every angle and in both signs.
     *
     * Provable rather than sampled: along a straight line the ratio is
     * constant, so an incumbent is by construction already the larger of the
     * two and can never be beaten by [ShellMetrics.REDIRECT] times itself.
     */
    @Test
    fun `a straight drag means exactly what it always did`() {
        for (degrees in 0 until 360 step 5) {
            val radians = degrees * PI.toFloat() / 180f
            val end = Pair(300f * cos(radians), 300f * sin(radians))
            val run = drive(leg(0f to 0f, end, steps = 60), 3)
            assertEquals(
                "a straight drag at $degrees° changed its meaning",
                ShellGesture.axis(end.first, end.second),
                run.second.axis,
            )
            assertEquals("a straight drag at $degrees° changed its mind", 0, run.third)
        }
    }

    /**
     * **The diagonal keeps whatever the gesture already is**, and without the
     * hysteresis a jittering diagonal changes its answer on nearly every frame.
     *
     * The memoryless rule is counted below as the counter-example it is, so
     * this test says what changed rather than merely what is.
     */
    @Test
    fun `the diagonal keeps whatever the gesture already is`() {
        assertEquals(ShellAxis.HORIZONTAL, ShellGesture.lean(100f, 100f, ShellAxis.HORIZONTAL))
        assertEquals(ShellAxis.VERTICAL, ShellGesture.lean(100f, 100f, ShellAxis.VERTICAL))
        // tan 54.5° = 1.4, so a hair past it the horizontal lets go and a hair
        // short of it does not.
        assertEquals(ShellAxis.VERTICAL, ShellGesture.lean(100f, 141f, ShellAxis.HORIZONTAL))
        assertEquals(ShellAxis.HORIZONTAL, ShellGesture.lean(100f, 139f, ShellAxis.HORIZONTAL))
        assertEquals(ShellAxis.HORIZONTAL, ShellGesture.lean(141f, 100f, ShellAxis.VERTICAL))
        assertEquals(ShellAxis.VERTICAL, ShellGesture.lean(139f, 100f, ShellAxis.VERTICAL))

        val jittered = (1..120).map { i ->
            Pair(i.toFloat() + if (i % 2 == 0) 1f else -1f, i.toFloat())
        }
        var memoryless = 0
        var last: ShellAxis? = null
        for ((dx, up) in jittered) {
            val answer = ShellGesture.axis(dx, up)
            if (answer != null && answer != last) memoryless++
            if (answer != null) last = answer
        }
        assertTrue(
            "the rule with no memory answers a different axis on nearly every frame",
            memoryless > 100,
        )
        assertEquals(0, drive(jittered, 3).third)
    }

    /**
     * A gesture that has moved is never a tap again, however far back toward
     * the origin it comes.
     *
     * The trap in re-asking a question that used to be asked once: the tap is
     * the ABSENCE of an axis, so a rule that could answer null a second time
     * would toggle the column under a finger that asked for neither.
     */
    @Test
    fun `a gesture that has moved is never a tap again`() {
        assertEquals(ShellAxis.HORIZONTAL, ShellGesture.lean(0f, 0f, ShellAxis.HORIZONTAL))
        assertEquals(ShellAxis.VERTICAL, ShellGesture.lean(0f, 0f, ShellAxis.VERTICAL))
        val run = drive(leg(0f to 0f, 60f to 0f) + leg(60f to 0f, 0f to 0f), 3)
        assertEquals(ShellAxis.HORIZONTAL, run.second.axis)
    }

    /**
     * **Once the page is in your hand the lean stops being asked.** The bound
     * on the whole change, and the one thing the old lock was protecting.
     *
     * Latched rather than recomputed: the second run brings the card back DOWN
     * into the column, which is a thumb lowering rather than a thumb letting
     * go.
     */
    @Test
    fun `the lift claims both axes once the page is in your hand`() {
        assertEquals(
            ShellAxis.VERTICAL,
            ShellGesture.lean(900f, 10f, ShellAxis.VERTICAL, holdingPage = true),
        )
        val carried = drive(leg(0f to 0f, 0f to 260f) + leg(0f to 260f, -500f to 260f), 3)
        assertEquals(ShellAxis.VERTICAL, carried.second.axis)
        assertTrue(carried.first.holdingPage)
        assertEquals(-500f, carried.second.sideways, 0.01f)

        val lowered = drive(leg(0f to 0f, 0f to 260f) + leg(0f to 260f, -300f to 100f), 3)
        assertEquals(ShellAxis.VERTICAL, lowered.second.axis)
        assertEquals(0, lowered.third)
    }

    /**
     * **A handover charges the claimed lift exactly what the page would have
     * jumped, and not a point more.**
     *
     * Two halves, two rules, because of what each channel is. The track is a
     * POSITION and re-bases whole. The column is not: it is shut, then whole,
     * and its row is read off the finger's absolute place on the glass. What is
     * a position on the vertical is the page's own rise past the last row,
     * which is [ShellGesture.pageRise].
     */
    @Test
    fun `a handover charges the lift only for what would have moved the page`() {
        // Inside the column: nothing to charge, and the menu opens under the
        // finger the moment the gesture turns.
        val inside = drive(leg(0f to 0f, -60f to 0f) + leg(-60f to 0f, -60f to 120f), 3)
        assertEquals(ShellAxis.VERTICAL, inside.second.axis)
        assertEquals(0f, inside.first.spentLift, 0.01f)
        assertEquals(120f, inside.second.lift, 0.01f)

        // A one-tab workspace is where an uncharged handover was worst: 44dp of
        // column, so the same turn would have thrown the page most of the way
        // into the overview in a single frame.
        val single = drive(leg(0f to 0f, -60f to 0f) + leg(-60f to 0f, -60f to 120f), 1)
        assertEquals(40f, single.first.spentLift, 2f)
        assertEquals(
            "the page starts the overview's run at nothing, not halfway through it",
            0f,
            ShellGesture.overviewProgress(84f - single.first.spentLift, 1),
            0.001f,
        )
    }

    /**
     * A charge handed back is a charge dropped.
     *
     * [ShellBarDrag.spentLift] describes what the VERTICAL was given, so it
     * means nothing while the horizontal holds the gesture; left standing it is
     * a phantom thumb some number of dp above a column nobody is in.
     *
     * The path is the one shape that can reach it, and it is narrow on purpose.
     * A charge is only non-zero when the vertical claims a gesture already past
     * the last row — and the charge puts the lift back ON the last row, which
     * is not yet [ShellGesture.pageIsHeld], so the page is not in your hand and
     * the lean is still being asked. One dp more of RISE and it would be held
     * and this could never happen; so the finger turns sideways instead.
     */
    @Test
    fun `a charge on the lift is dropped when the horizontal takes the gesture back`() {
        val run = drive(
            leg(0f to 0f, -143f to 0f) +
                leg(-143f to 0f, -143f to 201f) +
                leg(-143f to 201f, -400f to 201f),
            3,
        )
        assertEquals(ShellAxis.HORIZONTAL, run.second.axis)
        assertEquals(2, run.third)
        assertTrue("the page never left the display", !run.first.holdingPage)
        assertEquals(0f, run.first.spentLift, 0.01f)
        // The sideways charge is not dropped but RE-TAKEN: it is however far
        // sideways the gesture had gone the last time it changed its mind.
        assertTrue(run.first.spentSideways < -281f && run.first.spentSideways > -290f)
    }

    // MARK: - The column, and the row that was one off

    /**
     * **The bug, as reported: tapping a row in the drag-up menu did nothing.**
     *
     * The column unfurls UPWARD from the bar, so the row nearest the bar is the
     * LAST tab and the arithmetic has to count from the bottom and then invert.
     * Counting the other way puts every tap one column away from where it looks,
     * and on a two-tab workspace it puts every tap on the wrong tab.
     */
    @Test
    fun `the column row nearest the bar is the last tab`() {
        // Four tabs, 44dp each, the mapping shifted down by an 11dp bias. Just
        // above the bar is row 3; near the top is 0.
        assertEquals(3, ShellGesture.columnRow(above = 10f, tabCount = 4))
        assertEquals(2, ShellGesture.columnRow(above = 61f, tabCount = 4))
        assertEquals(1, ShellGesture.columnRow(above = 105f, tabCount = 4))
        assertEquals(0, ShellGesture.columnRow(above = 170f, tabCount = 4))
    }

    @Test
    fun `a tap outside the column lands on nothing`() {
        assertNull(ShellGesture.columnRow(above = 0f, tabCount = 4))
        assertNull(ShellGesture.columnRow(above = -5f, tabCount = 4))
        // 4 * 44 = 176 is the top edge, and the bias leaves an 11dp margin
        // above it so the top row keeps a full row of its own; past THAT the
        // finger has left the menu and the page has begun to rise.
        assertEquals(0, ShellGesture.columnRow(above = 187f, tabCount = 4))
        assertNull(ShellGesture.columnRow(above = 188f, tabCount = 4))
        assertNull(ShellGesture.columnRow(above = 20f, tabCount = 0))
    }

    /**
     * **The row is the one BELOW the fingertip, never the one above it.**
     *
     * A thumb covers the 44dp row it is on, so the row being chosen is the one
     * that cannot be seen. The bias is a quarter row, which is small enough
     * that the middle 33dp of every row still selects itself — a deliberate aim
     * is never overridden — and large enough to move the highlight clear of the
     * contact point.
     */
    @Test
    fun `the chosen row sits below the finger and never above it`() {
        val tabs = 4
        var below = 0
        var above = 1
        while (above <= 176) {
            val drawn = tabs - 1 - min(tabs - 1, ((above - 0.001f) / 44f).toInt())
            val chosen = ShellGesture.columnRow(above = above.toFloat(), tabCount = tabs)
            assertTrue(
                "at $above the row must be the one under the finger or the one below it",
                chosen == drawn || chosen == drawn + 1,
            )
            if (chosen == drawn + 1) below++
            val intoRow = above % 44
            if (intoRow > ShellMetrics.ROW_BIAS) {
                assertEquals("a finger clear of the bias band selects its own row", drawn, chosen)
            }
            above++
        }
        assertTrue("the bias has to move the answer somewhere", below > 0)
    }

    /**
     * **The row a DRAG chooses is the row under the finger, wherever the drag
     * started.** The bug iOS reported, and it was worth exactly one row.
     *
     * There used to be a second mapping — `columnSelection(up, tabCount)`,
     * `tabCount - ceil(up / 44)`, a pure delta with no idea where the finger
     * went down. Write `d` for how far below the column's bottom edge the touch
     * landed and the two agree only at `d == 0`. Here the fingertip is held at
     * one height and the drag's START is moved down through the bar row: the
     * same finger, over the same drawn row, used to choose two different tabs.
     */
    @Test
    fun `the chosen row follows the finger and not how far it has travelled`() {
        val fleet = ShellFleet(
            listOf(workspace("alpha", listOf(tab("a0"), tab("a1"), tab("a2"))))
        )
        val at = ShellPosition(0, 0)
        val above = 60f
        val row = ShellGesture.columnRow(above = above, tabCount = 3)
        assertEquals(1, row)

        fun oldDeltaMapping(up: Float): Int? {
            if (up < ShellMetrics.OPEN_MIN) return null
            val steps = min(3, kotlin.math.ceil(up / 44f).toInt())
            return if (steps > 0) 3 - steps else null
        }

        val oldAnswers = mutableSetOf<Int?>()
        for (d in listOf(0f, 11f, 22f, 33f, 44f)) {
            assertEquals(
                "the finger is over tab 1's row, so a release chooses tab 1",
                ShellRelease.Land(1),
                fleet.barRelease(axis = ShellAxis.VERTICAL, dx = 0f, up = above + d, at = at, row = row),
            )
            oldAnswers.add(oldDeltaMapping(above + d))
        }
        assertTrue(
            "the mapping this replaced gave different rows for one fingertip",
            oldAnswers.size > 1,
        )
        assertTrue("and at the bottom of the bar it was a whole row out", oldAnswers.contains(0))
    }

    @Test
    fun `the column is all or nothing, and pinning ignores the drag`() {
        assertEquals(0f, ShellGesture.columnHeight(up = 5f, tabCount = 3, pinned = false))
        assertEquals(132f, ShellGesture.columnHeight(up = 20f, tabCount = 3, pinned = false))
        // Pinned open, with no drag at all — the two are separate facts and
        // deriving one from the other is what made a tap toggle the wrong way.
        assertEquals(132f, ShellGesture.columnHeight(up = 0f, tabCount = 3, pinned = true))
    }

    // MARK: - The page, and the overview

    @Test
    fun `the page does not lift until the column is fully out`() {
        assertEquals(0f, ShellGesture.pageRise(up = 100f, tabCount = 3))
        assertTrue(!ShellGesture.pageIsHeld(up = 131f, tabCount = 3))
        assertEquals(20f, ShellGesture.pageRise(up = 152f, tabCount = 3))
        assertTrue(ShellGesture.pageIsHeld(up = 133f, tabCount = 3))
    }

    @Test
    fun `overview progress runs zero to one across the over-run and clamps`() {
        assertEquals(0f, ShellGesture.overviewProgress(up = 132f, tabCount = 3))
        assertEquals(0.5f, ShellGesture.overviewProgress(up = 170f, tabCount = 3), 1e-4f)
        assertEquals(1f, ShellGesture.overviewProgress(up = 208f, tabCount = 3))
        assertEquals(1f, ShellGesture.overviewProgress(up = 900f, tabCount = 3))
    }

    /**
     * **The bug, as reported: swiping down on the grid reopened the last
     * workspace.**
     *
     * The grid's pull-down read whether it was at the top at the RELEASE.
     * Scrolling back up through forty cards ends at the top with a large
     * downward translation — the same release, character for character, that a
     * deliberate pull-down produces. So it has to be gated on where the gesture
     * BEGAN.
     */
    @Test
    fun `scrolling the grid back to the top does not dismiss it`() {
        // Began mid-list, ended at the top having travelled a long way down.
        assertTrue(!ShellGesture.overviewDismisses(begunAtTop = false, down = 600f))
        // Began at the top and pulled.
        assertTrue(ShellGesture.overviewDismisses(begunAtTop = true, down = 600f))
        // Began at the top but barely moved.
        assertTrue(!ShellGesture.overviewDismisses(begunAtTop = true, down = 20f))
    }

    // MARK: - Thresholds

    @Test
    fun `a page turn commits at seventy and not before`() {
        assertTrue(!ShellGesture.commits(69.9f))
        assertTrue(ShellGesture.commits(70f))
        assertTrue(ShellGesture.commits(-70f))
    }

    @Test
    fun `left goes to the next and right to the previous`() {
        assertEquals(ShellDirection.NEXT, ShellGesture.direction(-1f))
        assertEquals(ShellDirection.PREVIOUS, ShellGesture.direction(1f))
        assertNull(ShellGesture.direction(0f))
        assertEquals(-1f, ShellDirection.NEXT.trackSign)
    }

    @Test
    fun `resistance applies only where there is nothing to bring in`() {
        assertEquals(100f, ShellGesture.translation(100f, rubberBanding = false))
        assertEquals(34f, ShellGesture.translation(100f, rubberBanding = true), 1e-4f)
    }

    // MARK: - Stepping

    @Test
    fun `the content walks one flat sequence across the fleet`() {
        val start = ShellPosition(0, 0)
        val within = simple.step(start, ShellDirection.NEXT, ShellTrack.CONTENT)!!
        assertEquals(ShellPosition(0, 1), within.position)
        assertTrue("staying inside a workspace is not a crossing", !within.crossesWorkspace)

        val across = simple.step(ShellPosition(0, 1), ShellDirection.NEXT, ShellTrack.CONTENT)!!
        assertEquals(ShellPosition(1, 0), across.position)
        assertTrue(across.crossesWorkspace)
    }

    /** Going back lands on the PREVIOUS workspace's last tab, so it reverses. */
    @Test
    fun `stepping back and forward returns you to where you were`() {
        val here = ShellPosition(1, 0)
        val back = simple.step(here, ShellDirection.PREVIOUS, ShellTrack.CONTENT)!!
        assertEquals(ShellPosition(0, 1), back.position)
        val forward = simple.step(back.position, ShellDirection.NEXT, ShellTrack.CONTENT)!!
        assertEquals(here, forward.position)
    }

    @Test
    fun `the ends of the fleet have nowhere to go and rubber-band`() {
        assertNull(simple.step(ShellPosition(0, 0), ShellDirection.PREVIOUS, ShellTrack.CONTENT))
        assertNull(simple.step(ShellPosition(1, 1), ShellDirection.NEXT, ShellTrack.CONTENT))
        assertTrue(
            simple.rubberBands(ShellPosition(0, 0), ShellDirection.PREVIOUS, ShellTrack.CONTENT))
        assertTrue(
            !simple.rubberBands(ShellPosition(0, 0), ShellDirection.NEXT, ShellTrack.CONTENT))
    }

    /** A workspace with no tabs is not a place a page turn can land. */
    @Test
    fun `an empty workspace is stepped over rather than into`() {
        val fleet = ShellFleet(
            listOf(
                workspace("alpha", listOf(tab("a0"))),
                workspace("empty", emptyList()),
                workspace("gamma", listOf(tab("g0"))),
            )
        )
        val step = fleet.step(ShellPosition(0, 0), ShellDirection.NEXT, ShellTrack.CONTENT)!!
        assertEquals(ShellPosition(2, 0), step.position)
    }

    /** The bar moves whole workspaces, and lands where that one was left. */
    @Test
    fun `the bar steps by workspace and resumes the remembered tab`() {
        val fleet = ShellFleet(
            listOf(
                workspace("alpha", listOf(tab("a0"), tab("a1"))),
                workspace("beta", listOf(tab("b0"), tab("b1"), tab("b2")), resume = 2),
            )
        )
        val step = fleet.step(ShellPosition(0, 1), ShellDirection.NEXT, ShellTrack.BAR)!!
        assertEquals(ShellPosition(1, 2), step.position)
        assertTrue(step.crossesWorkspace)
    }

    /** A remembered tab that has since exited falls back to the first. */
    @Test
    fun `a stale resume degrades to the first tab rather than crashing`() {
        val fleet = ShellFleet(
            listOf(
                workspace("alpha", listOf(tab("a0"))),
                workspace("beta", listOf(tab("b0")), resume = 7),
            )
        )
        assertEquals(
            ShellPosition(1, 0),
            fleet.step(ShellPosition(0, 0), ShellDirection.NEXT, ShellTrack.BAR)!!.position,
        )
    }

    /**
     * A crossing between runners is carried so the incoming pane can say so
     * before you commit to it. iOS holds this field nil because a `Connection`
     * is one runner over there; here the sequence really can leave the machine
     * you are on, and a swipe that quietly did would be the gesture doing
     * something bigger than it looks.
     */
    @Test
    fun `a step that changes machine says so`() {
        val fleet = ShellFleet(
            listOf(
                workspace("alpha", listOf(tab("a0")), runner = "laptop"),
                workspace("beta", listOf(tab("b0")), runner = "laptop"),
                workspace("gamma", listOf(tab("g0")), runner = "buildbox"),
            )
        )
        val sameMachine = fleet.step(ShellPosition(0, 0), ShellDirection.NEXT, ShellTrack.CONTENT)!!
        assertTrue(sameMachine.crossesWorkspace)
        assertTrue("alpha and beta are on one runner", !sameMachine.crossesRunner)

        val other = fleet.step(ShellPosition(1, 0), ShellDirection.NEXT, ShellTrack.CONTENT)!!
        assertTrue(other.crossesRunner)

        val viaBar = fleet.step(ShellPosition(1, 0), ShellDirection.NEXT, ShellTrack.BAR)!!
        assertTrue("the bar crosses machines too", viaBar.crossesRunner)
    }

    // MARK: - Releases

    @Test
    fun `a tap on the bar toggles the column`() {
        assertEquals(
            ShellRelease.ToggleColumn,
            simple.barRelease(axis = null, dx = 0f, up = 0f, at = ShellPosition(0, 0)),
        )
    }

    /**
     * **The bug, as reported: tapping a row in the drag-up menu did nothing.**
     *
     * The column declares no target of its own, so a tap on a row arrives here
     * with no axis — indistinguishable from a tap on the bar — and resolved to
     * `ToggleColumn`, which SHUT the menu the person was trying to use. Opening
     * had always worked because opening IS the toggle, which is why only a real
     * phone found it.
     */
    @Test
    fun `a tap on a column row lands on that row and does not shut the menu`() {
        val release = simple.barRelease(
            axis = null, dx = 0f, up = 0f, at = ShellPosition(0, 0), row = 1)
        assertEquals(ShellRelease.Land(1), release)
    }

    @Test
    fun `a committed sideways drag on the bar turns the page`() {
        val release = simple.barRelease(
            axis = ShellAxis.HORIZONTAL, dx = -80f, up = 0f, at = ShellPosition(0, 0))
        assertEquals(ShellPosition(1, 0), (release as ShellRelease.Commit).step.position)
    }

    @Test
    fun `a sideways drag that stops short springs back`() {
        assertEquals(
            ShellRelease.SpringBack,
            simple.barRelease(
                axis = ShellAxis.HORIZONTAL, dx = -60f, up = 0f, at = ShellPosition(0, 0)),
        )
        // Committed, but at the end of the fleet: nowhere to go is also a spring.
        assertEquals(
            ShellRelease.SpringBack,
            simple.barRelease(
                axis = ShellAxis.HORIZONTAL, dx = 80f, up = 0f, at = ShellPosition(0, 0)),
        )
    }

    @Test
    fun `dragging past the over-run opens the overview`() {
        // Two tabs: 88 of column, then 76 more.
        assertEquals(
            ShellRelease.OpenOverview,
            simple.barRelease(
                axis = ShellAxis.VERTICAL, dx = 0f, up = 164f, at = ShellPosition(0, 0)),
        )
    }

    /** Held the page and swiped: arrive at the neighbour still holding it. */
    @Test
    fun `lifting the page and swiping carries it to the next workspace`() {
        val release = simple.barRelease(
            axis = ShellAxis.VERTICAL, dx = -80f, up = 164f, at = ShellPosition(0, 0))
        assertEquals(ShellPosition(1, 0), (release as ShellRelease.Carry).step.position)
    }

    /**
     * **A fast upward fling reaches the overview and does NOT carry.** The
     * owner: *"when I fling the workspace up, quite often it animates the
     * workspace to the n-1th or n+1th grid square… if my fling is angled too
     * much it picks either the previous or next workspace to land on."*
     *
     * A thumb's arc deviates about 0.18 of its travel sideways, but its
     * TANGENT at the release leans twice as far — 0.36 — so a 3000 dp/s fling
     * leaves at 1090 sideways, which projects 544 dp past a `dx` of 24 that
     * could never have committed. See [ShellGesture.carried].
     */
    @Test
    fun `a fast angled fling reaches the overview without carrying`() {
        assertEquals(
            ShellRelease.OpenOverview,
            simple.barRelease(
                axis = ShellAxis.VERTICAL, dx = -24f, up = 96f, at = ShellPosition(0, 0),
                dxVelocity = -1090f, upVelocity = 3000f),
        )
    }

    /**
     * And the gesture the carry EXISTS for still works: a lifted page flicked
     * across, from short of the 70 dp that commits on translation alone. The
     * gate is on the momentum, never on the drawing.
     */
    @Test
    fun `a lifted page flicked sideways still carries`() {
        val release = simple.barRelease(
            axis = ShellAxis.VERTICAL, dx = -40f, up = 164f, at = ShellPosition(0, 0),
            dxVelocity = -1500f, upVelocity = 0f)
        assertEquals(ShellPosition(1, 0), (release as ShellRelease.Carry).step.position)
    }

    @Test
    fun `a vertical drag that reveals nothing costs nothing`() {
        assertEquals(
            ShellRelease.Abandon,
            simple.barRelease(
                axis = ShellAxis.VERTICAL, dx = 0f, up = 4f, at = ShellPosition(0, 0)),
        )
    }

    /**
     * **A downward flick on the bar costs nothing**, which is the same guarantee
     * the old wrong axis sign was reaching for and arriving at from the other
     * side. Down is negative `up`, `pageRise` floors at zero and there is no
     * column row above a finger that is below the bar, so the release is
     * `Abandon` — not a page turn.
     */
    @Test
    fun `a downward flick on the bar costs nothing`() {
        assertEquals(
            ShellRelease.Abandon,
            simple.barRelease(
                axis = ShellAxis.VERTICAL, dx = 40f, up = -300f, at = ShellPosition(0, 0)),
        )
    }

    /**
     * A vertical drag on a PANE belongs to the pane, which is scrolling. This is
     * the release-level statement of the axis bug: even a large horizontal
     * component must not turn the page once the gesture is vertical.
     */
    @Test
    fun `scrolling a pane never turns the page`() {
        assertEquals(
            ShellRelease.SpringBack,
            simple.contentRelease(axis = ShellAxis.VERTICAL, dx = -200f, at = ShellPosition(0, 0)),
        )
        assertEquals(
            ShellRelease.SpringBack,
            simple.contentRelease(axis = null, dx = -200f, at = ShellPosition(0, 0)),
        )
    }

    @Test
    fun `a committed swipe on a pane steps the flat sequence`() {
        val release =
            simple.contentRelease(axis = ShellAxis.HORIZONTAL, dx = -90f, at = ShellPosition(0, 0))
        assertEquals(ShellPosition(0, 1), (release as ShellRelease.Commit).step.position)
    }

    // MARK: - Momentum

    /**
     * The throw distance is a scroll view's own, to the point, and the same
     * number iOS uses.
     *
     * `AgentKit`'s `ShellGesture.project` is this function, and
     * `theProjectionIsAScrollViewsOwnThrowDistance` is these numbers: a
     * thousand units a second coasts 499, so a flick here travels exactly as
     * far as a flick in every other scroller on the device. The two platforms
     * throw the same distance or the shell is two shells.
     */
    @Test
    fun `the projection is a scroll view's own throw distance`() {
        assertEquals(0.998f, ShellGesture.DECELERATION_RATE)
        // A tenth of a unit of tolerance, and it is `Float` rather than
        // slack: `1f - 0.998f` is 0.0020000339 in single precision, so the
        // quotient lands 0.017 short of 998 on this side of the port and
        // exactly on it in Swift's `CGFloat`. A twentieth of a millimetre of
        // throw, and asserting past it would be asserting about IEEE 754.
        assertEquals(499f, ShellGesture.project(1000f), 0.1f)
        assertEquals(998f, ShellGesture.project(2000f), 0.1f)
        assertEquals(-499f, ShellGesture.project(-1000f), 0.1f)
        assertEquals(0f, ShellGesture.project(0f))
        // A rate that is not a decay is not a projection, and zero is the only
        // answer that cannot make a release worse.
        assertEquals(0f, ShellGesture.project(1000f, decelerationRate = 1f))
        assertEquals(0f, ShellGesture.project(1000f, decelerationRate = 0f))
        // Nothing moving projects nowhere, so every default here is the old
        // behaviour exactly.
        assertEquals(40f, ShellGesture.projected(40f, 0f))
    }

    /**
     * **A flick up from the bar reaches the overview even though the thumb
     * lifted over a menu row.**
     *
     * The talk's PIP example reproduced as the *before* case: *"the issue here
     * is that we're only looking at position, we're completely ignoring the
     * momentum."* The overview used to be reachable only by dragging a full
     * 76dp past the last row and stopping there, so a flick — which is how
     * anybody who has used a task switcher asks for a grid — landed on
     * whichever row the thumb was passing.
     *
     * The negative controls are the point rather than a footnote: the same
     * finger in the same place, released with no momentum and with a little,
     * still lands on the row it is over.
     */
    @Test
    fun `a flick up from over a menu row escapes to the overview`() {
        val at = ShellPosition(0, 0)  // two tabs, so 88 of column and 164 to escape
        val up = 60f
        val row = ShellGesture.columnRow(above = up, tabCount = 2)
        assertEquals(0, row)
        fun release(velocity: Float) = simple.barRelease(
            axis = ShellAxis.VERTICAL, dx = 0f, up = up, at = at, row = row,
            upVelocity = velocity,
        )
        assertEquals(ShellRelease.OpenOverview, release(1500f))
        assertEquals("a finger that stopped chose the row it stopped on", ShellRelease.Land(0), release(0f))
        assertEquals("and one still drifting has not asked to leave", ShellRelease.Land(0), release(100f))
    }

    /**
     * **A short fast flick across a pane turns the page**, and the same forty
     * units placed deliberately still does not.
     *
     * The sideways half of the same defect. Forty units is a flick anybody
     * would make and is nowhere near `PAGE_COMMIT`, so a pane was something you
     * could only leave by a long laborious drag — the talk's own
     * counter-example: *"those same swipes wouldn't get you very far… you'd
     * have to do these long, laborious swipes."*
     */
    @Test
    fun `a short fast flick across a pane turns the page`() {
        val at = ShellPosition(0, 0)
        val flicked = simple.contentRelease(
            axis = ShellAxis.HORIZONTAL, dx = -40f, at = at, dxVelocity = -600f)
        assertEquals(ShellPosition(0, 1), (flicked as ShellRelease.Commit).step.position)
        assertEquals(
            "the same forty units, placed rather than thrown, is still nothing",
            ShellRelease.SpringBack,
            simple.contentRelease(axis = ShellAxis.HORIZONTAL, dx = -40f, at = at, dxVelocity = 0f),
        )
        assertEquals(
            "and a vertical lock still belongs to the pane, however fast",
            ShellRelease.SpringBack,
            simple.contentRelease(
                axis = ShellAxis.VERTICAL, dx = -200f, at = at, dxVelocity = -3000f),
        )
    }

    /**
     * The direction comes off the THROW, not off the translation.
     *
     * A drag one way flicked back the other way at the last moment is asking to
     * go where it is heading. Reading `commits` off the projection and
     * `direction` off the raw translation would turn the page backwards, which
     * is a worse answer than the spring-back it replaced.
     */
    @Test
    fun `a drag flicked back the other way turns the page the way it is headed`() {
        val release = simple.contentRelease(
            axis = ShellAxis.HORIZONTAL, dx = 30f, at = ShellPosition(0, 1), dxVelocity = -1500f)
        assertEquals(ShellPosition(1, 0), (release as ShellRelease.Commit).step.position)
    }

    // MARK: - The overview's order

    @Test
    fun `precedence is needs-you, then an unread diff, then silence, then work`() {
        assertEquals(
            ShellPrecedence.NEEDS_YOU,
            workspace("w", listOf(tab("a"), tab("b", wants = true))).precedence,
        )
        assertEquals(
            ShellPrecedence.UNREAD_DIFF,
            workspace("w", listOf(diffTab("d", unread = true), tab("a"))).precedence,
        )
        assertEquals(
            ShellPrecedence.ALL_STALE,
            workspace("w", listOf(tab("a", stale = true), tab("b", stale = true))).precedence,
        )
        assertEquals(
            ShellPrecedence.WORKING,
            workspace("w", listOf(tab("a", stale = true), tab("b"))).precedence,
        )
        assertEquals(ShellPrecedence.WORKING, workspace("w", emptyList()).precedence)
    }

    /**
     * **A finished turn sorts with needs-you even though it draws the quieter
     * review ring.** `wantsAttention` is blocked OR done, and it is this app's
     * single definition of "should this interrupt someone", shared with the Mac
     * since long before the glance vocabulary. What a mark SAYS and what a list
     * SORTS BY are different questions; a workspace whose agent just finished is
     * exactly what you opened the app to see.
     *
     * **The tab used to carry no mark at all here**, an `AgentOutcome.DONE` and
     * a null mark, because the review tier refused an agent's state. It carries
     * the review ring now, and the sort must not have quietly moved down a rung
     * with the drawing — which is the whole reason `wantsAttention` is a
     * separate field rather than something derived from the mark.
     */
    @Test
    fun `a finished agent pulls its workspace to the top while wearing the review ring`() {
        val done = ShellTab(
            id = "d",
            title = "claude",
            mark = GlanceMark(GlanceMark.Attention.TO_REVIEW, GlanceMark.Core.AT_A_PROMPT),
            wantsAttention = true,
        )
        val w = workspace("w", listOf(done))
        assertEquals(ShellPrecedence.NEEDS_YOU, w.precedence)
        assertEquals(
            "and it draws the review ring, a rung below where it sorts",
            GlanceMark.Attention.TO_REVIEW,
            w.tabs.first().mark?.attention,
        )
        assertNull("a turn that merely ended is not an outcome", w.tabs.first().outcome)
    }

    @Test
    fun `the overview sorts by rung and keeps fleet order inside one`() {
        val fleet = ShellFleet(
            listOf(
                workspace("quiet-a", listOf(tab("q0"))),
                workspace("blocked", listOf(tab("b0", wants = true))),
                workspace("quiet-b", listOf(tab("q1"))),
                workspace("review", listOf(diffTab("d", unread = true))),
            )
        )
        assertEquals(listOf(1, 3, 0, 2), fleet.overviewOrder())
    }

    @Test
    fun `search filters the same order and a blank query filters nothing`() {
        val fleet = ShellFleet(
            listOf(
                workspace("auth-refactor", listOf(tab("a"))),
                workspace("schema-migrate", listOf(tab("b", wants = true))),
            )
        )
        assertEquals(listOf(1, 0), fleet.overviewOrder("   "))
        assertEquals(listOf(0), fleet.overviewOrder("AUTH"))
        assertEquals(emptyList<Int>(), fleet.overviewOrder("nothing"))
    }

    // MARK: - Lookup

    @Test
    fun `a fleet answers about places that exist and refuses ones that do not`() {
        assertTrue(simple.contains(ShellPosition(1, 1)))
        assertTrue(!simple.contains(ShellPosition(1, 2)))
        assertTrue(!simple.contains(ShellPosition(9, 0)))
        assertEquals("b1", simple.tab(ShellPosition(1, 1))?.id)
        assertNull(simple.tab(ShellPosition(9, 0)))
        assertEquals(ShellPosition(1, 0), simple.position("b0"))
        assertNull(simple.position("nope"))
        assertEquals(ShellPosition(0, 0), simple.first)
        assertNull(ShellFleet(emptyList()).first)
        assertEquals(2, simple.tabCount(0))
        assertEquals(0, simple.tabCount(9))
    }

    /** A fleet whose leading workspaces are empty still finds a first tab. */
    @Test
    fun `the first tab skips empty workspaces`() {
        val fleet = ShellFleet(
            listOf(workspace("empty", emptyList()), workspace("real", listOf(tab("r"))))
        )
        assertEquals(ShellPosition(1, 0), fleet.first)
    }

    // MARK: - The track, and the commit that must not move anything

    /**
     * **The no-bounce guarantee, as arithmetic.**
     *
     * Committing a page turn changes two things at once: which pane is current,
     * and the track's offset. If those two writes disagree by so much as a
     * frame, the page visibly jumps back — which is the bug the web prototype
     * works around with two `requestAnimationFrame`s and iOS works around with a
     * completion callback and a transaction that disables animations.
     *
     * This asserts that they cannot disagree, because the pair is a pure
     * re-parameterisation: after a commit, **every** pane is drawn at exactly
     * the pixel it was drawn at before — not just the incoming one. So the swap
     * frame is identical to the frame before it and there is nothing for a race
     * to be wrong about.
     */
    @Test
    fun committingMovesNothing() {
        val width = 412f
        for (offset in listOf(0f, -30f, -90f, 45f, 200f, -411f)) {
            for (direction in ShellDirection.entries) {
                val after = ShellTrackGeometry.residual(offset, direction, width)
                // Every slot the track can draw, not only the one arriving.
                for (slot in -2..2) {
                    val before = ShellTrackGeometry.x(slot, width, offset)
                    // The commit moves the current index one step, so a pane
                    // that was at `slot` is now at `slot + trackSign`.
                    val nowAt = ShellTrackGeometry.x(
                        slot + direction.trackSign.toInt(), width, after)
                    assertEquals(
                        "offset=$offset $direction slot=$slot moved",
                        before, nowAt, 1e-3f,
                    )
                }
            }
        }
    }

    /**
     * The residual is a whole page, in the direction that undoes the index move.
     * Stated separately from [committingMovesNothing] because that test would
     * also pass if both halves were zero.
     */
    @Test
    fun `the residual is one page against the direction of travel`() {
        assertEquals(412f, ShellTrackGeometry.residual(0f, ShellDirection.NEXT, 412f))
        assertEquals(-412f, ShellTrackGeometry.residual(0f, ShellDirection.PREVIOUS, 412f))
        // Dragged 90 to the left and released: the incoming pane is 322 to the
        // right of centre and settles from there rather than from a page away.
        assertEquals(322f, ShellTrackGeometry.residual(-90f, ShellDirection.NEXT, 412f))
    }

    /**
     * Dragging left brings the NEXT pane in from the right. A sign error here is
     * invisible in a screenshot and obvious in a hand: the page would come from
     * the wrong side.
     */
    @Test
    fun `dragging left brings the next pane in from the right`() {
        val width = 400f
        // At rest the neighbours are exactly one page off-screen either side.
        assertEquals(400f, ShellTrackGeometry.x(1, width, 0f))
        assertEquals(-400f, ShellTrackGeometry.x(-1, width, 0f))
        // Dragging left (negative) pulls the next one toward centre.
        assertEquals(300f, ShellTrackGeometry.x(1, width, -100f))
        assertEquals(-100f, ShellTrackGeometry.x(0, width, -100f))
        assertEquals(ShellDirection.NEXT, ShellGesture.direction(-100f))
    }

    /** The rail tracks a measured page rather than a constant one. */
    @Test
    fun `the rail is the page less its two insets`() {
        assertEquals(361f, ShellMetrics.railWidth(393f))
        assertEquals(380f, ShellMetrics.railWidth(412f))
    }
}
