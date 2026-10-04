package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** When a worktree says large files weren't downloaded, and who may try again (ov-199): AgentKit's `LfsNoticeTests`. */
class LfsNoticeTest {
    @Test
    fun `a worktree with no pointer files says nothing`() {
        assertNull(LfsNotice.make(null, mayAct = true))
        assertNull(LfsNotice.make(0, mayAct = true))
    }

    @Test
    fun `a worktree with pointer files says so, and a read grant gets the sentence without the button`() {
        assertEquals(LfsNotice(2, canRetry = true), LfsNotice.make(2, mayAct = true))
        assertFalse(LfsNotice.make(2, mayAct = false)!!.canRetry)
    }

    @Test
    fun `the words are the card's`() {
        assertEquals("Some large files weren't downloaded.", LfsNotice.TITLE)
        assertTrue(LfsNotice.DETAIL.contains("git lfs pull"))
        assertEquals("Try again", LfsNotice.RETRY)
    }
}
