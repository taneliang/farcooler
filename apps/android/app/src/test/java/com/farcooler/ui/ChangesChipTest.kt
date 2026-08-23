package com.farcooler.ui

import com.farcooler.model.InboxRow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What the Changes tab says before there is a review to show.
 *
 * Phase 5 of the parity program builds the diff surface; what this phase owed
 * was a chip that does something honest in the meantime, and "leave it dead"
 * was explicitly not on the table. So the words are pinned here rather than in a
 * screenshot nobody takes: three states that are genuinely different, and one of
 * them is the one a phone gets wrong.
 */
class ChangesChipTest {
    private fun row(insertions: Int, deletions: Int) =
        InboxRow(workspaceId = "w", insertions = insertions, deletions = deletions)

    /**
     * A runner that has not answered is not a branch with nothing on it.
     *
     * Null is the absence of a fact; a row with no diff is the fact that there
     * is nothing. `model/NeedsYou.kt` draws the same line and it matters more
     * here, where reading the two as one means telling somebody their branch is
     * clean because their runner is still shaking hands.
     */
    @Test
    fun anUnansweredRunnerAndACleanBranchDoNotGetTheSameSentence() {
        assertNotEquals(changesTabMessage(null), changesTabMessage(row(0, 0)))
        assertTrue(changesTabMessage(null).contains("Waiting"))
    }

    /** The one clause only this surface carries: the counts include uncommitted work. */
    @Test
    fun theTabSaysWhatTheCountsActuallyCount() {
        assertTrue(changesTabMessage(row(82, 13)).contains("committed or not"))
    }

    /**
     * There is no hover on a phone, so the accessibility label is the only place
     * the chip itself can say what its numbers mean.
     */
    @Test
    fun theChipTellsAScreenReaderWhatItIsCounting() {
        assertEquals("Changes", changesDescription(null))
        assertEquals("Changes", changesDescription(row(0, 0)))
        assertEquals(
            "Changes, 82 added, 13 removed, including work that isn’t committed yet",
            changesDescription(row(82, 13)),
        )
    }
}
