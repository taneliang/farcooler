package com.farcooler.ui

import com.farcooler.model.BoardPage
import com.farcooler.model.Plan
import com.farcooler.net.PageListState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** What the Plan view's Pages section shows (ov-285, review M1): never "Pages 0" over a read that failed. */
class PagesSectionTest {
    private val plan = Plan()

    @Test
    fun `a read that failed draws the notice with no count`() {
        val section = PagesSection.of(PageListState.Unavailable, plan)
        assertEquals(PagesSection.Unavailable, section)
        assertNull("a count above a read that failed", section.count)
    }

    @Test
    fun `pages read draw their count, and none or none read draw nothing`() {
        val listed = PagesSection.of(PageListState.Loaded(listOf(BoardPage("1", "train", "Train"), BoardPage("2", "spend", "Spend"))), plan)
        assertEquals(2, listed.count)
        assertEquals(PagesSection.None, PagesSection.of(PageListState.Loaded(emptyList()), plan))
        assertEquals(PagesSection.None, PagesSection.of(null, plan))
    }
}
