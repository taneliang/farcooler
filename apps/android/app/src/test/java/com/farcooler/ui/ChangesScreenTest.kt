package com.farcooler.ui

import com.farcooler.model.ChangeCommit
import com.farcooler.model.ChangeSet
import com.farcooler.model.ChangedFile
import com.farcooler.model.ChangesState
import com.farcooler.model.DiffScope
import com.farcooler.model.ReviewPosition
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The sentences the diff surface says, and the ones it must not.
 *
 * **Nothing on this screen has drawn a frame** — there is no emulator and no
 * device for any of phase 5 — so every string worth being sure about is built by
 * a pure function out here rather than inline in a composable, and read by a
 * test rather than by a screenshot nobody took. That is the shape
 * `changesDescription` and `NeedsYouScreen`'s reassurance copy already use.
 *
 * What is deliberately NOT here: anything about layout, colour, scrolling or
 * hit targets. None of it can be asserted from the JVM and pretending otherwise
 * would be worse than saying so.
 */
class ChangesScreenTest {
    private fun file(
        path: String,
        status: String = "modified",
        plus: Int = 0,
        minus: Int = 0,
        binary: Boolean = false,
    ) = ChangedFile(
        path = path,
        statusWord = status,
        insertions = plus,
        deletions = minus,
        binary = binary,
    )

    // ---- which nothing this is ----

    /**
     * The three empties are three different facts, and the commit one is the
     * only one that can be got badly wrong.
     *
     * A commit is compared against its FIRST parent — `Selector::Commit` in the
     * daemon's `file_diff.rs` — so a merge that only joined two branches
     * genuinely changed nothing against that side while changing plenty against
     * the other. "This commit is empty" would be a claim this screen is in no
     * position to make; naming the comparison is a fact.
     */
    @Test
    fun `an empty comparison says which comparison was empty`() {
        assertEquals(
            "This branch hasn’t committed anything yet.",
            nothingHere(DiffScope.Branch),
        )
        assertEquals(
            "Nothing uncommitted. The workspace is clean.",
            nothingHere(DiffScope.Local),
        )
        val commit = nothingHere(DiffScope.Commit("a".repeat(40)))
        assertTrue(commit.contains("first parent"))
        assertTrue(commit.contains("clean merge"))
    }

    // ---- coming back ----

    /**
     * A subject rather than a sha wherever one is known.
     *
     * "You were reading push.ts in *handle retries on 429*" is a place somebody
     * recognizes; `local/a1b2c3d4` is a place they have to decode.
     */
    @Test
    fun `the bookmark is described in the branch's own words`() {
        val sha = "a1b2c3d4" + "0".repeat(32)
        val commits = listOf(ChangeCommit(sha = sha, subject = "handle retries on 429"))
        assertEquals(
            "You were at push.ts, in “handle retries on 429”.",
            resumeDescription(
                ReviewPosition(scope = sha, file = "src/net/push.ts"),
                commits,
            ),
        )
    }

    /**
     * The commit was amended or rebased away, which for an agent-authored
     * branch is not an edge case. The sha is still the honest answer — it is
     * what was written down — and eight characters is what a person can carry
     * back to a terminal.
     */
    @Test
    fun `a commit the branch has forgotten is still named`() {
        val sha = "a1b2c3d4" + "0".repeat(32)
        assertEquals(
            "You were at push.ts, in commit a1b2c3d4.",
            resumeDescription(ReviewPosition(scope = sha, file = "src/net/push.ts"), emptyList()),
        )
    }

    /**
     * The list-of-headings case, which is most of the first window of a review:
     * nothing open, and the file at the top of the screen is what "how far down
     * was I" means. It reads the same as an open file on purpose — from the
     * reader's side both mean "you were about here".
     */
    @Test
    fun `a scrolled position with nothing open still says where`() {
        assertEquals(
            "You were at Cargo.lock, in the uncommitted work.",
            resumeDescription(
                ReviewPosition(scope = DiffScope.Local.wire, topFile = "Cargo.lock"),
                emptyList(),
            ),
        )
        assertEquals(
            "You were on the whole branch.",
            resumeDescription(ReviewPosition(scope = DiffScope.Branch.wire), emptyList()),
        )
    }

    // ---- spoken ----

    /**
     * Read aloud, the summary card's top line is "vs main", "plus 82", "minus
     * 13" — three fragments that never say which comparison they are the total
     * of, and the control that would have said it is a separate element further
     * down. So the label names it, the same fix the rows that lead here got.
     */
    @Test
    fun `the counts say aloud what they are the total of`() {
        val branch = ChangesState(changeSet = ChangeSet(baseRef = "main"))
        assertEquals(
            "Branch, against main, 82 added, 13 removed",
            spokenComparison(branch, 82, 13),
        )
        assertEquals(
            "Uncommitted, against HEAD, 4 added, 0 removed",
            spokenComparison(branch.copy(scope = DiffScope.Local), 4, 0),
        )
    }

    /**
     * A file heading is read on the way into every file, so the directory is
     * left out of it: `crates/daemon/src` spelled letter by letter once per file
     * on a forty-file branch is what `NeedsYou`'s workspace header already
     * refuses. The directory stays on screen for the eye, and the index sheet —
     * where a screen-reader user CHOOSES a file rather than passes one — will
     * say the whole path when phase 5c builds it.
     */
    @Test
    fun `a file heading is spoken without its directory`() {
        assertEquals(
            "push.ts, Modified, 12 added, 3 removed",
            spokenHeading(file("src/net/push.ts", plus = 12, minus = 3)),
        )
        assertEquals(
            "logo.png, Added, binary",
            spokenHeading(file("assets/logo.png", status = "added", binary = true)),
        )
    }

    /**
     * A daemon too old to send the field decodes as UNKNOWN rather than as a
     * wrong letter, so the row draws a bullet — and the spoken label has to say
     * the only thing that is actually known.
     */
    @Test
    fun `a file with no status is Changed rather than a guess`() {
        assertEquals("a.rs, Changed", spokenHeading(file("a.rs", status = "")))
    }

    // ---- counted things ----

    @Test
    fun `nothing on this screen says 1 files`() {
        assertEquals("1 generated file", generatedHeading(1))
        assertEquals("4 generated files", generatedHeading(4))
        assertEquals("plus 1 generated file", generatedAside(1))
        assertEquals("plus 4 generated files", generatedAside(4))
        assertEquals("1 commit", commitCount(1))
        assertEquals("12 commits", commitCount(12))
        assertEquals("1 unchanged line", unchangedLines(1))
        assertEquals("9 unchanged lines", unchangedLines(9))
        assertEquals("Show 1 more line", remainingLines(1))
        assertEquals("Show 3412 more lines", remainingLines(3412))
    }
}
