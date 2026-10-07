package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/** The ledger applies pages and follow diffs by id (AgentKit's `AgentRowLedger`). */
class AgentRowLedgerTest {
    private fun prose(id: String, ord: Long, rev: Long = ord, text: String = id) =
        AgentRow(id, ord, rev, kind = AgentRow.Kind.OfProse(AgentRow.Prose(text)))

    private fun page(epoch: Long, rev: Long, vararg rows: AgentRow, more: Boolean = false) =
        AgentRowPage(epoch, rev, more, rows.toList())

    @Test
    fun `the fixture's follow applies in place - only what changed crosses`() {
        val ledger = AgentRowLedger()
        ledger.replace(AgentRowPage.decode(AgentRowFixture.page))
        val before = ledger.held().associateBy { it.id }

        val followed = ledger.apply(AgentRowChanges.decode(AgentRowFixture.follow))
        val delta = (followed as AgentRowFollowed.Changed).delta

        assertEquals(listOf("prose:2", "tool:toolu_1"), delta.rows.map { it.id })
        assertEquals(listOf("queued:1"), delta.removed)
        val ids = ledger.held().map { it.id }
        assertEquals(delta.order, ids)
        assertTrue("prose:2" in ids && "queued:1" !in ids)
        // The row that didn't change is the same object, so a list keyed by id skips it.
        assertSame(before.getValue("ask:1"), ledger.held().first { it.id == "ask:1" })
        assertEquals(12L, ledger.cursor?.second)
    }

    @Test
    fun `paging again keeps the object of every row that did not change`() {
        val ledger = AgentRowLedger()
        ledger.replace(AgentRowPage.decode(AgentRowFixture.page))
        val before = ledger.held()
        // A page after a failure or a reset: the same rows, decoded afresh.
        val delta = ledger.replace(AgentRowPage.decode(AgentRowFixture.page))
        assertEquals(emptyList<AgentRow>(), delta.rows)
        ledger.held().zip(before).forEach { (now, was) -> assertSame(was, now) }
    }

    @Test
    fun `a follow from another epoch or a reset says to page again`() {
        val ledger = AgentRowLedger()
        ledger.replace(page(1, 5, prose("a", 0)))
        assertEquals(AgentRowFollowed.Reset, ledger.apply(AgentRowChanges(2, 6, false, emptyList())))
        assertEquals(AgentRowFollowed.Reset, ledger.apply(AgentRowChanges(1, 6, true, emptyList())))
    }

    @Test
    fun `a page of the same projection keeps the older rows it doesn't cover`() {
        val ledger = AgentRowLedger()
        ledger.replace(page(1, 5, prose("a", 0), prose("b", 1), prose("c", 2), more = true))
        ledger.replace(page(1, 7, prose("c", 2, rev = 6), prose("d", 3), more = true))
        assertEquals(listOf("a", "b", "c", "d"), ledger.held().map { it.id })
        // A page from a new projection replaces everything.
        ledger.replace(page(2, 1, prose("z", 0)))
        assertEquals(listOf("z"), ledger.held().map { it.id })
    }

    @Test
    fun `an older page goes above, and one from another projection is dropped`() {
        val ledger = AgentRowLedger()
        ledger.replace(page(1, 5, prose("c", 2), prose("d", 3), more = true))
        assertEquals(2L, ledger.oldestOrd)
        ledger.older(page(2, 5, prose("x", 0)))
        assertEquals(listOf("c", "d"), ledger.held().map { it.id })
        ledger.older(page(1, 5, prose("a", 0), prose("b", 1), prose("c", 2)))
        assertEquals(listOf("a", "b", "c", "d"), ledger.held().map { it.id })
        assertEquals(0L, ledger.oldestOrd)
    }

    @Test
    fun `a row older than the window is someone else's page and isn't drawn`() {
        val ledger = AgentRowLedger()
        ledger.replace(page(1, 5, prose("c", 2)))
        ledger.apply(AgentRowChanges(1, 6, false, listOf(AgentRowChanges.Change.Insert(prose("old", 0)))))
        assertEquals(listOf("c"), ledger.held().map { it.id })
        assertNull(AgentRowLedger().cursor)
    }
}
