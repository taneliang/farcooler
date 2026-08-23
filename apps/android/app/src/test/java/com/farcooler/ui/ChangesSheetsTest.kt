package com.farcooler.ui

import androidx.compose.runtime.saveable.SaverScope
import com.farcooler.model.BranchRef
import com.farcooler.model.ChangeCommit
import com.farcooler.model.ChangeSet
import com.farcooler.model.ChangedFile
import com.farcooler.model.ReviewAnchor
import com.farcooler.model.SentReviewBatch
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What the review's five sheets filter by, and what they say.
 *
 * **Nothing in this phase has drawn a frame** — there is no emulator and no
 * device for any of phase 5 — so every rule worth being sure about is a pure
 * function beside its sheet and read here, which is the same shape
 * `ChangesScreenTest` and `NeedsYouScreen`'s reassurance copy already take.
 *
 * What is deliberately NOT here: anything about layout, keyboard, colour or hit
 * targets. None of it can be asserted from the JVM and pretending otherwise
 * would be worse than saying so.
 */
class ChangesSheetsTest {
    private fun file(path: String) = ChangedFile(path = path, statusWord = "modified")

    /**
     * Everything a `Saver` needs from Compose, which is one predicate.
     *
     * `SaverScope` asks whether a value can go in a `Bundle`; this record is one
     * string, so the answer is always yes and no framework is needed to say so.
     * That is the point of the record being a string in the first place.
     */
    private val saverScope = SaverScope { true }

    private fun round(sheet: ReviewSheet?): ReviewSheet? {
        val saved = with(ReviewSheetSaver) { with(saverScope) { save(sheet) } } ?: return null
        return ReviewSheetSaver.restore(saved)
    }

    // ---- the file index ----

    /**
     * The whole path, not the leaf.
     *
     * "daemon" is how somebody asks for everything under `crates/daemon/`, and a
     * filter that matched only the filename would answer that with nothing —
     * which on a forty-file branch is the one query the sheet exists for.
     */
    @Test
    fun `the file filter matches the directory as well as the name`() {
        val files = listOf(
            file("crates/daemon/src/review_ops.rs"),
            file("crates/core/src/feed.rs"),
            file("apps/android/app/src/main/java/com/farcooler/ui/ChangesScreen.kt"),
        )
        assertEquals(
            listOf("crates/daemon/src/review_ops.rs"),
            matchingFiles(files, "daemon").map { it.path },
        )
        assertEquals(3, matchingFiles(files, "   ").size)
        // Case does not survive a thumb.
        assertEquals(1, matchingFiles(files, "CHANGESSCREEN").size)
    }

    /**
     * **A heading is passed; a row of the index is chosen**, and 5b wrote that
     * split down before there was a sheet to put it in. `spokenHeading` leaves
     * the directory out because it is read on the way into every one of forty
     * files; this says it, because two files called `mod.rs` are two identical
     * rows without it — which is the one thing an index may not be.
     */
    @Test
    fun `an index row is spoken with its directory, unlike a heading`() {
        val changed = ChangedFile(
            path = "crates/daemon/src/review_ops.rs",
            statusWord = "modified",
            insertions = 12,
            deletions = 3,
        )
        assertEquals(
            "review_ops.rs, in crates/daemon/src, Modified, 12 added, 3 removed",
            spokenIndexRow(changed),
        )
        // A file at the root has no aside to add, and inventing one would be the
        // row reading out a directory nobody has.
        assertEquals(
            spokenHeading(file("README.md")),
            spokenIndexRow(file("README.md")),
        )
    }

    /** Two different facts, and only one of them is about the filter. */
    @Test
    fun `an empty index says whether it is the comparison or the filter`() {
        assertEquals("Nothing changed in this comparison.", noFilesHere(comparisonIsEmpty = true))
        assertEquals("No files match that.", noFilesHere(comparisonIsEmpty = false))
        assertEquals("This project has no branches yet.", noBranchesHere(repositoryIsEmpty = true))
        assertEquals("No branches match that.", noBranchesHere(repositoryIsEmpty = false))
    }

    // ---- the history ----

    /**
     * Subject, body, author — and the sha as a PREFIX.
     *
     * The body is in there because it is often where the word somebody
     * remembers actually appears: an agent puts the file it touched in the
     * rationale far more often than in the subject. The sha is a prefix because
     * nobody searches for the middle of a hash, and a substring match on hex
     * turns every two-character query into noise.
     */
    @Test
    fun `the commit filter reads all four things people half-remember`() {
        val commit = ChangeCommit(
            sha = "a1b2c3d4" + "e".repeat(32),
            subject = "handle retries on 429",
            body = "The push path in push.ts gave up after one attempt.",
            author = "claude",
        )
        val other = ChangeCommit(sha = "f".repeat(40), subject = "tidy", author = "e-liang")
        val commits = listOf(commit, other)

        assertEquals(1, matchingCommits(commits, "retries").size)
        assertEquals(1, matchingCommits(commits, "push.ts").size)
        assertEquals(1, matchingCommits(commits, "claude").size)
        assertEquals(1, matchingCommits(commits, "a1b2").size)
        // The middle of a hash is noise, not a query.
        assertTrue(matchingCommits(commits, "c3d4").isEmpty())
        assertEquals(2, matchingCommits(commits, "").size)
    }

    /**
     * The base is empty until the first read lands, and "Every commit since ,
     * at once" is how an unloaded pane would read it out.
     */
    @Test
    fun `the whole-branch row names the base only when it knows one`() {
        assertEquals("Every commit since main, at once", wholeBranchDescription("main"))
        assertEquals("Every commit on this branch, at once", wholeBranchDescription(""))
    }

    // ---- the base ----

    /**
     * **A guessed base has to read differently from a recorded one**, because
     * that difference is the entire reason this sheet exists: a guess is the one
     * base that silently produces a wrong diff looking exactly like a right one,
     * and a screen that said "compared against main" for both would make them
     * indistinguishable at the moment somebody came to check.
     */
    @Test
    fun `the base sheet says where the current base came from`() {
        fun described(source: String, ref: String = "main") =
            baseDescription(ChangeSet(baseRef = ref, baseSource = source))

        assertEquals("Pinned to main.", described("recorded"))
        assertTrue(described("guessed").contains("Guessed"))
        assertTrue(described("guessed").contains("may be wrong"))
        assertTrue(described("upstream").contains("upstream"))
        assertTrue(described("pr_base").contains("pull request"))
        assertTrue(described("default_branch").contains("default branch"))
        // An arm nobody has written a sentence for still says something true.
        assertEquals("Compared against main.", described("something_new"))
        // Before the first read lands there is no ref to name.
        assertEquals(
            "Nothing is recorded as this worktree’s base yet.",
            described("guessed", ref = ""),
        )
    }

    /**
     * `main` and `origin/main` are different answers to "what is this based on",
     * and a list of thirty names is exactly where they would otherwise be told
     * apart by nothing.
     */
    @Test
    fun `a branch row says where it lives and whether somebody is on it`() {
        assertEquals(
            "local · checked out · handle retries",
            branchAside(
                BranchRef(name = "main", local = true, checkedOut = true, subject = "handle retries")
            ),
        )
        assertEquals("remote", branchAside(BranchRef(name = "origin/main", remote = true)))
        assertEquals(
            "local and remote",
            branchAside(BranchRef(name = "main", local = true, remote = true)),
        )
    }

    // ---- the outbox ----

    /** A note's place, short enough for a row and long enough to be a place. */
    @Test
    fun `a pending note says which file and where in it`() {
        assertEquals(
            "push.ts · lines 120-148",
            notePlace(ReviewAnchor(file = "src/net/push.ts", firstLine = 120, lastLine = 148)),
        )
        // A comment on a whole file has no line to name, and inventing one would
        // be the row claiming an anchor the note does not carry.
        assertEquals("push.ts", notePlace(ReviewAnchor(file = "src/net/push.ts")))
    }

    @Test
    fun `the counts are said in words and pluralized`() {
        assertEquals("1 note", noteCount(1))
        assertEquals("4 notes", noteCount(4))
        assertEquals("1 note for the agent", notesWaiting(1))
        assertEquals("3 notes for the agent", notesWaiting(3))
        assertEquals("1 file", fileCount(1))
        assertEquals("30 files", fileCount(30))
    }

    /**
     * **The two verbs are two different promises and must not read the same.**
     * A send was handed to an agent with no way to confirm it arrived; a batch
     * put in a composer is sitting in a text field waiting for a person to press
     * Send, and calling that "sent" would be the app claiming something nobody
     * did — on the one screen whose whole design is about not being able to
     * claim delivery.
     */
    @Test
    fun `a receipt says which of the two things happened`() {
        val now = 1_755_900_000L
        fun batch(placed: Boolean?, at: Long = now - 600) = SentReviewBatch(
            text = "…", agentName = "claude", sentAt = at, count = 2, placedInComposer = placed,
        )

        assertEquals("sent to claude · 10m ago", receiptDetail(batch(null), now))
        assertEquals("sent to claude · 10m ago", receiptDetail(batch(false), now))
        assertEquals("put in claude’s composer · 10m ago", receiptDetail(batch(true), now))
    }

    /**
     * An age rather than a wall clock. A clock time would have to be formatted
     * in some time zone, and the one question a receipt answers is "was that
     * this window or the last one".
     */
    @Test
    fun `how long ago is said in the shorthand the rest of the app uses`() {
        val now = 1_755_900_000L
        assertEquals("just now", handedOverWhen(now - 5, now))
        assertEquals("2m ago", handedOverWhen(now - 120, now))
        assertEquals("3h ago", handedOverWhen(now - 10_800, now))
        assertEquals("2d ago", handedOverWhen(now - 172_800, now))
    }

    /** A phrase written for mid-sentence use, starting one. */
    @Test
    fun `the anchor's place reads as a sentence above the composer`() {
        assertEquals("Lines 12-40", capitalizedFirst("lines 12-40"))
        assertEquals("The whole file", capitalizedFirst("the whole file"))
        assertEquals("", capitalizedFirst(""))
    }

    // ---- which sheet is up ----

    /**
     * **The composer's anchor survives the process, because what is in the
     * composer is a sentence somebody typed.** `ReviewCommentQueue` makes that
     * promise from the moment a note is ADDED; this is the window before that,
     * which on a phone Android is free to kill between two ninety-second windows
     * is exactly where the kill lands.
     */
    @Test
    fun `the open sheet round trips, anchor and all`() {
        assertEquals(ReviewSheet.History, round(ReviewSheet.History))
        assertEquals(ReviewSheet.Index, round(ReviewSheet.Index))
        assertEquals(ReviewSheet.Outbox, round(ReviewSheet.Outbox))
        assertEquals(ReviewSheet.Base, round(ReviewSheet.Base))
        assertEquals(null, round(null))

        val anchor = ReviewAnchor(
            file = "src/net/push.ts",
            commit = "a".repeat(40),
            firstLine = 120,
            lastLine = 148,
            quote = "  await fetch(url)",
        )
        assertEquals(ReviewSheet.Note(anchor), round(ReviewSheet.Note(anchor)))
    }

    /**
     * A record this build cannot read is a sheet nobody asked for, and opening
     * the wrong one over a diff is worse than opening none.
     */
    @Test
    fun `an unreadable record opens no sheet`() {
        assertEquals(null, ReviewSheetSaver.restore("note:{not json"))
        assertEquals(null, ReviewSheetSaver.restore("something a later build wrote"))
    }
}
