package com.farcooler.ui

import com.farcooler.model.InboxRow
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * What the Changes chip says about a worktree it has not opened.
 *
 * This file used to pin three sentences as well — the tab's own body, which said
 * how much had changed and that reading it here was not built yet. **Phase 5b
 * built it, so those sentences and their two tests are gone rather than
 * adjusted.** They were honest for exactly as long as they were true, and a copy
 * test that outlives the copy is a test asserting the app still says something
 * false. What survives is the part that was never about the deferral: the chip's
 * own spoken label.
 */
class ChangesChipTest {
    private fun row(insertions: Int, deletions: Int) =
        InboxRow(workspaceId = "w", insertions = insertions, deletions = deletions)

    /**
     * There is no hover on a phone, so the accessibility label is the only place
     * the chip itself can say what its numbers mean — and they are not the
     * branch total the workspace list shows under Branch.
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

    /**
     * The front door's review row and the tab it opens describe one worktree
     * the same way.
     *
     * They are the same target on two screens, and until that row actually
     * opened the diff there was nothing to compare it against. The lead word
     * differs because the row is a verb and the chip is a place; everything
     * after the comma is the same string by construction, and this is what
     * makes that construction load-bearing rather than incidental.
     */
    @Test
    fun theReviewRowSaysWhatTheTabSays() {
        assertEquals("Review changes", changesDescription(null, lead = "Review changes"))
        assertEquals(
            "Review changes, 82 added, 13 removed, including work that isn’t committed yet",
            changesDescription(row(82, 13), lead = "Review changes"),
        )
        assertEquals(
            changesDescription(row(82, 13)).substringAfter("Changes"),
            changesDescription(row(82, 13), lead = "Review changes")
                .substringAfter("Review changes"),
        )
    }
}
