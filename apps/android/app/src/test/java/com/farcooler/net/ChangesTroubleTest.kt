package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * What the Changes tab says when a read fails (ov-148).
 *
 * Separate from `ChangesStoreTest`, which is large enough already.
 */
class ChangesTroubleTest {
    /**
     * A dropped link used to fall through to the generic arm, which carried the
     * core's raw transcript ("EPIPE") under a sentence about a request that
     * "didn't finish". The link dropping is a known cause and has its own words.
     */
    @Test
    fun `a dropped link on the whole worktree says so and carries no transcript`() {
        val trouble = ChangesStore.loadTrouble(DisconnectedException("EPIPE"))
        assertEquals(
            "The connection to this runner dropped. Try again once it’s back.",
            trouble.sentence,
        )
        assertNull(trouble.transcript)
    }

    @Test
    fun `an unexplained worktree failure still carries the runner's words`() {
        val trouble = ChangesStore.loadTrouble(CoreException("EPIPE"))
        assertEquals("EPIPE", trouble.transcript)
    }

    /**
     * The row under this sentence has a Try Again button, so the sentence must
     * not also tell the reader to reopen the file.
     */
    @Test
    fun `a failed file read points at the button and not at reopening`() {
        for (e in listOf(CoreException("boom"), DisconnectedException("gone"))) {
            assertFalse(ChangesStore.fileTrouble(e).contains("Open"))
        }
        assertEquals(
            "This file’s changes couldn’t be read.",
            ChangesStore.fileTrouble(CoreException("boom")),
        )
        assertEquals(
            "The connection to this runner dropped.",
            ChangesStore.fileTrouble(DisconnectedException("gone")),
        )
    }
}
