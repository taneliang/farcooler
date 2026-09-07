package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The arithmetic behind dragging a workspace card.
 *
 * The same cases as `WorkspaceOrderTests.swift` in AgentKit, deliberately: the
 * two apps drag the same cards into the same order on the same runner, and a
 * disagreement between them shows up as a card that lands in one place on a
 * phone and another on a Mac.
 *
 * Here rather than in a UI test because there is no UI test that could catch any
 * of it — this app has no Compose test dependency, and a drop that lands one
 * card short looks exactly like a drop that lands right.
 */
class WorkspaceOrderTest {

    private val list = listOf("a", "b", "c", "d")

    @Test
    fun `dropping above a card puts the dragged one in front of it`() {
        assertEquals(
            listOf("a", "d", "b", "c"),
            WorkspaceOrder.moved(list, "d", "b", WorkspaceOrder.Edge.ABOVE),
        )
    }

    @Test
    fun `dropping below a card puts the dragged one after it`() {
        assertEquals(
            listOf("b", "c", "a", "d"),
            WorkspaceOrder.moved(list, "a", "c", WorkspaceOrder.Edge.BELOW),
        )
    }

    /**
     * Dragging downwards is where a hand-written reorder goes wrong: the
     * target's index shifts when the dragged card is lifted out, and using the
     * index from before that lands the card one short of where it was dropped —
     * every time, in the same direction.
     */
    @Test
    fun `dragging downwards lands where it was dropped and not one short`() {
        assertEquals(
            listOf("b", "c", "d", "a"),
            WorkspaceOrder.moved(list, "a", "d", WorkspaceOrder.Edge.BELOW),
        )
        assertEquals(
            listOf("b", "c", "a", "d"),
            WorkspaceOrder.moved(list, "a", "d", WorkspaceOrder.Edge.ABOVE),
        )
        assertEquals(
            listOf("d", "a", "b", "c"),
            WorkspaceOrder.moved(list, "d", "a", WorkspaceOrder.Edge.ABOVE),
        )
    }

    /**
     * A move that changes nothing must come back identical, because that is how
     * a caller decides not to spend a round trip — and a reorder request that
     * changes nothing still makes every other client re-read the fleet.
     */
    @Test
    fun `a drop that would change nothing comes back unchanged`() {
        assertEquals(list, WorkspaceOrder.moved(list, "b", "b", WorkspaceOrder.Edge.ABOVE))
        assertEquals(list, WorkspaceOrder.moved(list, "b", "c", WorkspaceOrder.Edge.ABOVE))
        assertEquals(list, WorkspaceOrder.moved(list, "b", "a", WorkspaceOrder.Edge.BELOW))
        assertEquals(list, WorkspaceOrder.moved(list, "a", "b", WorkspaceOrder.Edge.ABOVE))
    }

    /**
     * A card the list does not hold cannot be placed by it. Not theoretical: the
     * fleet is re-read every few seconds, so a worktree can be removed by
     * another client mid-drag.
     */
    @Test
    fun `a stranger on either end leaves the list alone`() {
        assertEquals(list, WorkspaceOrder.moved(list, "z", "b", WorkspaceOrder.Edge.ABOVE))
        assertEquals(list, WorkspaceOrder.moved(list, "a", "z", WorkspaceOrder.Edge.BELOW))
        assertEquals(
            emptyList<String>(),
            WorkspaceOrder.moved(emptyList(), "a", "b", WorkspaceOrder.Edge.ABOVE),
        )
    }

    @Test
    fun `the edge is decided at the midpoint of the card`() {
        assertEquals(WorkspaceOrder.Edge.ABOVE, WorkspaceOrder.edge(0f, 40f))
        assertEquals(WorkspaceOrder.Edge.ABOVE, WorkspaceOrder.edge(19.9f, 40f))
        assertEquals(WorkspaceOrder.Edge.BELOW, WorkspaceOrder.edge(20f, 40f))
        assertEquals(WorkspaceOrder.Edge.BELOW, WorkspaceOrder.edge(40f, 40f))
        // A card that has not been measured yet must not fling anything to the end.
        assertEquals(WorkspaceOrder.Edge.ABOVE, WorkspaceOrder.edge(12f, 0f))
    }

    // ---- which card the finger is over ----
    //
    // A workspace in this list is a header followed by however many terminal
    // rows, so a card's extent runs from its own header to the next one's. A
    // finger halfway down a workspace's terminals is over THAT workspace, and
    // the guard below is the one that catches an implementation that measured
    // only the headers and left every gap between them dead.

    private val cards = listOf(
        WorkspaceOrder.Card("a", top = 0, bottom = 200),
        WorkspaceOrder.Card("b", top = 200, bottom = 260),
        WorkspaceOrder.Card("c", top = 260, bottom = 500),
    )

    @Test
    fun `the finger is over the workspace whose block it is in, terminals included`() {
        // Deep inside a's terminal rows, well past any header's height.
        assertEquals(WorkspaceOrder.Landing("a", WorkspaceOrder.Edge.ABOVE), WorkspaceOrder.landing(cards, 40))
        assertEquals(WorkspaceOrder.Landing("a", WorkspaceOrder.Edge.BELOW), WorkspaceOrder.landing(cards, 150))
        // And the short card between them, which is a header with nothing under it.
        assertEquals(WorkspaceOrder.Landing("b", WorkspaceOrder.Edge.ABOVE), WorkspaceOrder.landing(cards, 210))
        assertEquals(WorkspaceOrder.Landing("b", WorkspaceOrder.Edge.BELOW), WorkspaceOrder.landing(cards, 250))
    }

    // The lazy list's own measurements, which is where the extents above come
    // from in the app: `FleetScreen`'s `visibleItemsInfo` becomes [Laid], and
    // [WorkspaceOrder.cards] turns that into the list [landing] reads. A
    // workspace is a header plus its terminal rows, all siblings in one flat
    // `LazyColumn`, so the arithmetic that groups them is the part with a bug
    // available in it.
    private val laid = listOf(
        WorkspaceOrder.Laid("a", offset = 0, size = 60),
        WorkspaceOrder.Laid("a/1", offset = 60, size = 70),
        WorkspaceOrder.Laid("a/2", offset = 130, size = 70),
        WorkspaceOrder.Laid("b", offset = 200, size = 60),
        WorkspaceOrder.Laid("c", offset = 260, size = 60),
        WorkspaceOrder.Laid("c/1", offset = 320, size = 180),
    )
    private val headers = setOf("a", "b", "c")

    /**
     * The hand-written extents above are the ones the code actually produces.
     *
     * Everything below [cards] was checked against a list written out by hand,
     * so the function that builds that list from what the screen measured was
     * never run by any of it — and building it by measuring the headers alone,
     * which leaves every terminal row in a dead band, passed the entire file.
     * 320 of this fixture's 500 pixels are terminal rows, which is the ordinary
     * shape of the screen.
     */
    @Test
    fun `a card runs from its own header to the next one, terminals included`() {
        assertEquals(cards, WorkspaceOrder.cards(laid, headers))
    }

    /** And the two ends: a prefix before the first header, and the last row. */
    @Test
    fun `a header-less prefix is ignored and the last row ends the last card`() {
        val banner = WorkspaceOrder.Laid("banner", offset = -40, size = 40)
        assertEquals(cards, WorkspaceOrder.cards(listOf(banner) + laid, headers))
        // The final card ends at the bottom of the LAST item, not the bottom of
        // its own header — the terminals under the last workspace are as much
        // of it as the ones under the first.
        assertEquals(500, WorkspaceOrder.cards(laid, headers).last().bottom)
        assertEquals(emptyList<WorkspaceOrder.Card>(), WorkspaceOrder.cards(emptyList(), headers))
    }

    /** The same question as above, asked through what the screen measured. */
    @Test
    fun `a finger in a workspace's terminal rows is over that workspace`() {
        val measured = WorkspaceOrder.cards(laid, headers)
        assertEquals(WorkspaceOrder.Landing("a", WorkspaceOrder.Edge.ABOVE), WorkspaceOrder.landing(measured, 40))
        assertEquals(WorkspaceOrder.Landing("a", WorkspaceOrder.Edge.BELOW), WorkspaceOrder.landing(measured, 150))
        assertEquals(WorkspaceOrder.Landing("c", WorkspaceOrder.Edge.ABOVE), WorkspaceOrder.landing(measured, 300))
        assertEquals(WorkspaceOrder.Landing("c", WorkspaceOrder.Edge.BELOW), WorkspaceOrder.landing(measured, 450))
    }

    @Test
    fun `dragging past either end means the front or the back of the list`() {
        assertEquals(WorkspaceOrder.Landing("a", WorkspaceOrder.Edge.ABOVE), WorkspaceOrder.landing(cards, -400))
        assertEquals(WorkspaceOrder.Landing("c", WorkspaceOrder.Edge.BELOW), WorkspaceOrder.landing(cards, 9000))
        assertNull(WorkspaceOrder.landing(emptyList(), 10))
    }
}
