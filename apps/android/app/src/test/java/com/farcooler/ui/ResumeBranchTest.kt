package com.farcooler.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The one sentence that explains an operation with no form in front of it.
 *
 * Resuming collapses `NewWorkspaceSheet` to a branch name and a button, because
 * `Service::adopt_branch` ignores `task_name` and derives the worktree's name
 * itself. That makes this sentence the whole of what anybody is told before a
 * directory appears on their runner — there is no name field left to imply one,
 * and no base field to imply the other.
 */
class ResumeBranchTest {

    /**
     * The prompt names the branch, names the folder it becomes, and promises
     * the branch is untouched.
     *
     * The folder is the branch's LAST SEGMENT, which is `adopt_branch`'s own
     * `branch.rsplit('/').next()`. Deriving it here rather than saying "a
     * worktree named after it" is the difference between a sentence somebody
     * can check against their runner afterwards and one they cannot.
     */
    @Test
    fun `the adoption prompt names the folder the branch becomes`() {
        val prefixed = adoptionDescription("feat/rate-limiting")
        assertTrue(prefixed.contains("feat/rate-limiting"))
        // The prefix says what kind of work it is; the directory does not repeat
        // it, and this sentence is the only place that is ever explained.
        assertTrue(prefixed.contains("rate-limiting"))
        assertFalse(prefixed.contains("called feat/rate-limiting"))
        // The half that decides whether this is frightening, and it is true:
        // adoption checks a branch out and writes no commit.
        assertTrue(prefixed.contains("Nothing on the branch changes"))
    }

    /**
     * A branch with no prefix is its own folder name, and the sentence must not
     * read as though two different things are happening.
     */
    @Test
    fun `an unprefixed branch names the same thing twice`() {
        val plain = adoptionDescription("main")
        assertEquals(
            "Far Cooler takes main over in a new worktree called main. " +
                "Nothing on the branch changes.",
            plain,
        )
    }

    /**
     * Deep prefixes take the leaf, not the second segment.
     *
     * `rsplit('/').next()` on the runner takes everything after the LAST slash,
     * so this side has to as well — `substringAfterLast` and not
     * `substringAfter`. Getting that backwards would name a folder the runner
     * never creates, which is worse than saying nothing.
     */
    @Test
    fun `a deep branch name takes its last segment`() {
        assertTrue(adoptionDescription("user/e/feat/parser").contains("called parser"))
    }
}
