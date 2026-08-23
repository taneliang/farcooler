package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Everything the Changes tab derives, proven without a screen.
 *
 * This is the reason [ChangesState] is an immutable value rather than a dozen
 * flows on the store: which files a scope is about, where the generated ones go,
 * what `7 of 23` says, and — the part that is new to this platform — which INDEX
 * a path resolves to in the list a `LazyColumn` draws, are all pure functions of
 * one value. There is no emulator and no device for this phase and no UI to look
 * at, so a pure model is not tidiness; it is the difference between a proven
 * rule and a claimed one.
 */
class ChangesStateTest {
    private fun file(path: String, plus: Int = 1, minus: Int = 0) =
        ChangedFile(path = path, statusWord = "modified", insertions = plus, deletions = minus)

    private val branchFiles = listOf(
        file("crates/daemon/src/a.rs", plus = 100, minus = 20),
        file("Cargo.lock", plus = 4_000, minus = 12),
        file("crates/daemon/src/b.rs", plus = 200, minus = 100),
    )

    private val set = ChangeSet(
        branch = "feat/x",
        insertions = 4_300,
        deletions = 132,
        commits = listOf(
            ChangeCommit(sha = "aaa", subject = "first", insertions = 10, deletions = 1),
            ChangeCommit(sha = "bbb", subject = "second"),
            ChangeCommit(sha = "ccc", subject = "third"),
        ),
        files = branchFiles,
        workingTree = WorkingTree(
            staged = listOf("crates/core/src/feed.rs"),
            unstaged = listOf("crates/core/src/feed.rs", "README.md"),
            untracked = listOf("notes/new.md"),
            conflicted = emptyList(),
            changes = listOf(
                ChangedFile("crates/core/src/feed.rs", "modified", insertions = 4, deletions = 1),
                ChangedFile("crates/core/src/feed.rs", "modified", insertions = 9, deletions = 2),
                ChangedFile("README.md", "modified", insertions = 3),
                ChangedFile("notes/new.md", "untracked", insertions = 12),
            ),
        ),
    )

    private val state = ChangesState(changeSet = set)

    // ---- which files a scope is about ----

    @Test
    fun `branch shows what the branch committed`() {
        assertEquals(branchFiles.map { it.path }, state.files.map { it.path })
    }

    /**
     * A commit's file list is FETCHED rather than filtered out of the change set,
     * which carries per-file counts for the whole branch and nothing per-commit.
     */
    @Test
    fun `a commit shows only what that call answered`() {
        val commit = state.copy(
            scope = DiffScope.Commit("aaa"),
            commitFiles = listOf(file("crates/daemon/src/a.rs")),
        )
        assertEquals(listOf("crates/daemon/src/a.rs"), commit.files.map { it.path })
        // Empty until the round trip lands, which is why `commitUnreadable` is a
        // separate fact from an empty list.
        assertTrue(state.copy(scope = DiffScope.Commit("aaa")).files.isEmpty())
    }

    /**
     * Uncommitted comes from the working tree, in one order and never twice — and
     * an untracked file is in it, because a file an agent just wrote is the only
     * kind of file this scope exists to show.
     */
    @Test
    fun `uncommitted is staged then unstaged then conflicted then untracked, uniqued`() {
        val local = state.copy(scope = DiffScope.Local)
        assertEquals(
            listOf("crates/core/src/feed.rs", "README.md", "notes/new.md"),
            local.files.map { it.path },
        )
        assertEquals(ChangedFileStatus.UNTRACKED, local.files[2].status)
        assertTrue(local.isUntracked("notes/new.md"))
    }

    /**
     * A file that is staged and then modified again has a record in both groups
     * and both are counted — exact for every file in one group or the other, and
     * an upper bound only when the same lines are touched twice.
     */
    @Test
    fun `a path dirty in two groups sums its counts once`() {
        val local = state.copy(scope = DiffScope.Local)
        val feed = local.files.first { it.path == "crates/core/src/feed.rs" }
        assertEquals(13, feed.insertions)
        assertEquals(3, feed.deletions)
        assertEquals(4 + 9 + 3 + 12, local.uncommittedInsertions)
        assertEquals(1 + 2, local.uncommittedDeletions)
        // Not the branch's numbers, which answer a different question.
        assertEquals(4_300, local.changeSet.insertions)
    }

    /**
     * Nothing inside a commit is untracked — committing is what tracking IS — so
     * a file some commit changed and somebody has since deleted and rewritten is
     * still fetchable.
     */
    @Test
    fun `nothing is untracked inside a commit`() {
        val commit = state.copy(scope = DiffScope.Commit("aaa"))
        assertFalse(commit.isUntracked("notes/new.md"))
        assertTrue(state.copy(scope = DiffScope.Local).isUntracked("notes/new.md"))
    }

    // ---- what a tool wrote ----

    @Test
    fun `the lockfile is counted apart and read last`() {
        assertEquals(listOf("Cargo.lock"), state.generatedFiles.map { it.path })
        assertEquals(
            listOf("crates/daemon/src/a.rs", "crates/daemon/src/b.rs"),
            state.handWrittenFiles.map { it.path },
        )
        assertEquals(
            listOf("crates/daemon/src/a.rs", "crates/daemon/src/b.rs", "Cargo.lock"),
            state.reviewOrder.map { it.path },
        )
        // The number this whole split exists to stop showing, and the one it
        // shows instead.
        assertEquals(4_012, state.generatedLineCount)
        assertEquals(300, state.writtenInsertions)
        assertEquals(120, state.writtenDeletions)
        assertEquals(4_000, state.generatedInsertions)
    }

    /**
     * Three sources in order of how much they know. This branch has a lockfile in
     * it, so the headline is the hand-written subtotal — never the row's own
     * `--shortstat`, which counts the lockfile.
     */
    @Test
    fun `a commit's counts prefer the hand-written subtotal when a tool wrote part of it`() {
        val commit = state.copy(scope = DiffScope.Commit("aaa"), commitFiles = branchFiles)
        assertEquals(300 to 120, commit.commitCounts)
    }

    @Test
    fun `with nothing generated the daemon's own count is believed`() {
        val commit = state.copy(
            scope = DiffScope.Commit("aaa"),
            commitFiles = listOf(file("a.rs", plus = 1, minus = 1)),
        )
        // `aaa` carries 10/1 on the change set; the file list sums to 1/1. The
        // daemon's is right before the file list has even landed.
        assertEquals(10 to 1, commit.commitCounts)
    }

    @Test
    fun `with no count on the commit the file list is summed`() {
        val commit = state.copy(
            scope = DiffScope.Commit("bbb"),
            commitFiles = listOf(file("a.rs", plus = 7, minus = 3)),
        )
        assertEquals(7 to 3, commit.commitCounts)
    }

    @Test
    fun `a commit whose file list has not landed says nothing rather than zero`() {
        assertNull(state.copy(scope = DiffScope.Commit("bbb")).commitCounts)
    }

    // ---- moving through the files ----

    @Test
    fun `the position label counts the list the reader is looking at`() {
        assertEquals("3 files", state.positionLabel)
        assertEquals("2 of 3", state.copy(expandedFile = "crates/daemon/src/b.rs").positionLabel)
        // The lockfile is third in reading order even though it is second on the
        // wire. Next must not jump backwards up the screen.
        assertEquals("3 of 3", state.copy(expandedFile = "Cargo.lock").positionLabel)
        assertEquals("No files", ChangesState().positionLabel)
        assertEquals(
            "1 file",
            ChangesState(changeSet = ChangeSet(files = listOf(file("a.rs")))).positionLabel,
        )
    }

    @Test
    fun `Next begins a review as well as continuing one`() {
        assertTrue(state.hasNextFile)
        assertFalse(state.hasPreviousFile)
        val last = state.copy(expandedFile = "Cargo.lock")
        assertFalse(last.hasNextFile)
        assertTrue(last.hasPreviousFile)
        assertFalse(ChangesState().hasNextFile)
    }

    // ---- one commit at a time ----

    @Test
    fun `the picker is newest first and the itinerary is oldest first`() {
        assertEquals(listOf("aaa", "bbb", "ccc"), state.commitsInOrder.map { it.sha })
        assertEquals(listOf("ccc", "bbb", "aaa"), state.commitsNewestFirst.map { it.sha })
    }

    @Test
    fun `a commit knows where it is in the sequence`() {
        val second = state.copy(scope = DiffScope.Commit("bbb"))
        assertEquals(1, second.commitIndex)
        assertEquals("Commit 2 of 3", second.commitPositionLabel)
        assertEquals("ccc", second.nextCommit?.sha)
        assertEquals("aaa", second.previousCommit?.sha)
        assertEquals("second", second.selectedCommitInfo?.subject)
    }

    /**
     * An amend or a rebase mid-review takes a commit off the branch while its
     * diff stays readable. The header has to stop claiming a position it does not
     * have rather than inventing one.
     */
    @Test
    fun `a commit the branch no longer lists has no position and no subject`() {
        val gone = state.copy(scope = DiffScope.Commit("deadbeef"))
        assertNull(gone.commitIndex)
        assertNull(gone.commitPositionLabel)
        assertNull(gone.selectedCommitInfo)
        assertNull(gone.nextCommit)
        assertNull(gone.previousCommit)
    }

    /**
     * Next means one thing at a time: while there are files left it is "next
     * file", and exactly once per commit it becomes "next commit".
     */
    @Test
    fun `Next carries into the following commit only at the end of this one`() {
        val commit = state.copy(
            scope = DiffScope.Commit("aaa"),
            commitFiles = listOf(file("a.rs"), file("b.rs")),
        )
        assertFalse(commit.nextIsCommit)
        assertTrue(commit.copy(expandedFile = "b.rs").nextIsCommit)
        // On the branch there is no next commit to carry into.
        assertFalse(state.copy(expandedFile = "Cargo.lock").nextIsCommit)
        // And not from the last commit on the branch.
        assertFalse(
            state.copy(
                scope = DiffScope.Commit("ccc"),
                commitFiles = listOf(file("a.rs")),
                expandedFile = "a.rs",
            ).nextIsCommit
        )
    }

    // ---- the rows, which are what an index means ----

    /**
     * The layout the view draws IS this list, which is what makes
     * `animateScrollToItem` safe. See [Jump].
     */
    @Test
    fun `the row list is the summary, the hand-written files, the heading, then the rest`() {
        val rows = state.rows
        assertEquals(
            listOf(
                ChangesRow.TOP_KEY,
                "crates/daemon/src/a.rs",
                "crates/daemon/src/b.rs",
                ChangesRow.GENERATED_KEY,
                "Cargo.lock",
            ),
            rows.map { it.key },
        )
        val heading = rows[3] as ChangesRow.GeneratedHeading
        assertEquals(1, heading.count)
        assertEquals(4_012, heading.lines)
    }

    @Test
    fun `a jump resolves to the index of the row it names`() {
        assertEquals(0, state.indexOf(ChangesRow.TOP_KEY))
        assertEquals(2, state.indexOf("crates/daemon/src/b.rs"))
        // Four, not three: the generated heading is a row and an index that
        // skipped it would land the scroll one file short every time a branch
        // regenerated a lockfile.
        assertEquals(4, state.indexOf("Cargo.lock"))
        assertNull(state.indexOf("a/file/that/left.rs"))
    }

    @Test
    fun `a branch with nothing generated has no heading row`() {
        val plain = ChangesState(
            changeSet = ChangeSet(files = listOf(file("a.rs"), file("b.rs")))
        )
        assertEquals(listOf(ChangesRow.TOP_KEY, "a.rs", "b.rs"), plain.rows.map { it.key })
        assertEquals(2, plain.indexOf("b.rs"))
    }

    /** Everything generated is still every file, under a heading that says so. */
    @Test
    fun `a branch that is all lockfile still gets its heading`() {
        val all = ChangesState(
            changeSet = ChangeSet(files = listOf(file("Cargo.lock", plus = 5), file("go.sum")))
        )
        assertEquals(
            listOf(ChangesRow.TOP_KEY, ChangesRow.GENERATED_KEY, "Cargo.lock", "go.sum"),
            all.rows.map { it.key },
        )
        assertEquals(2, (all.rows[1] as ChangesRow.GeneratedHeading).count)
    }

    @Test
    fun `an empty diff is one row, so the top always has somewhere to land`() {
        assertEquals(listOf(ChangesRow.TOP_KEY), ChangesState().rows.map { it.key })
        assertEquals(0, ChangesState().indexOf(ChangesRow.TOP_KEY))
    }
}
