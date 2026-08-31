package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
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
        // Four tabs, 44dp each. Just above the bar is row 3; near the top is 0.
        assertEquals(3, ShellGesture.columnRow(above = 10f, tabCount = 4))
        assertEquals(2, ShellGesture.columnRow(above = 50f, tabCount = 4))
        assertEquals(1, ShellGesture.columnRow(above = 100f, tabCount = 4))
        assertEquals(0, ShellGesture.columnRow(above = 170f, tabCount = 4))
    }

    @Test
    fun `a tap outside the column lands on nothing`() {
        assertNull(ShellGesture.columnRow(above = 0f, tabCount = 4))
        assertNull(ShellGesture.columnRow(above = -5f, tabCount = 4))
        // 4 * 44 = 176 is the top edge; past it is not the column.
        assertNull(ShellGesture.columnRow(above = 177f, tabCount = 4))
        assertNull(ShellGesture.columnRow(above = 20f, tabCount = 0))
    }

    /**
     * Hovering during a DRAG counts the other way round from tapping, and both
     * have to be right: the drag reveals rows from the bottom up, so the first
     * row revealed is the last tab and the selection walks backwards as travel
     * grows.
     */
    @Test
    fun `dragging up selects backwards from the last tab`() {
        assertNull("nothing chosen inside the open threshold",
            ShellGesture.columnSelection(up = 10f, tabCount = 4))
        assertEquals(3, ShellGesture.columnSelection(up = 20f, tabCount = 4))
        assertEquals(2, ShellGesture.columnSelection(up = 50f, tabCount = 4))
        assertEquals(0, ShellGesture.columnSelection(up = 176f, tabCount = 4))
        // Past the top the selection holds at the first tab rather than running off.
        assertEquals(0, ShellGesture.columnSelection(up = 400f, tabCount = 4))
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
            axis = null, dx = 0f, up = 0f, at = ShellPosition(0, 0), tapRow = 1)
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
     * side. Down is negative `up`, `pageRise` floors at zero and
     * `columnSelection` refuses under the open threshold, so the release is
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

    /** The rail tracks a measured page rather than a constant one. */
    @Test
    fun `the rail is the page less its two insets`() {
        assertEquals(361f, ShellMetrics.railWidth(393f))
        assertEquals(380f, ShellMetrics.railWidth(412f))
    }
}
