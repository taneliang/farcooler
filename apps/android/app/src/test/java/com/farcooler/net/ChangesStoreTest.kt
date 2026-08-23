package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import com.farcooler.data.InMemoryReviewStorage
import com.farcooler.model.ChangeSet
import com.farcooler.model.ChangeCommit
import com.farcooler.model.ChangedFile
import com.farcooler.model.ChangesRow
import com.farcooler.model.DiffComputation
import com.farcooler.model.DiffHunkReply
import com.farcooler.model.DiffLineReply
import com.farcooler.model.DiffScope
import com.farcooler.model.FileDiffReply
import com.farcooler.model.ReviewAnchor
import com.farcooler.model.ReviewBookmarks
import com.farcooler.model.ReviewPosition
import com.farcooler.model.ReviewRef
import com.farcooler.model.WorkingTree
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The store, which is everything about a review that is not pure.
 *
 * There is no emulator, no device and no runner for this phase, and no UI at all
 * — so a fake [ChangesSource] standing in for one runner is the whole of what can
 * be exercised, and it is more than it sounds: the cache invalidation, the
 * generation guard on an answer that lands after the reader moved on, the
 * one-file prefetch, and the resume that has to land somewhere sensible when the
 * branch moved underneath it are all in here, and every one of them is a rule
 * that would be invisible on a screen until it was wrong.
 */
class ChangesStoreTest {
    private val ref = ReviewRef("host-a", "w1")

    /**
     * The scope the store launches its one prefetch on.
     *
     * Unconfined against the test's own scheduler rather than `backgroundScope`,
     * and that is a real trap rather than a preference: `advanceUntilIdle` does
     * NOT run background work — a background coroutine only gets a turn while
     * the foreground is waiting — so a prefetch launched there never runs and
     * every assertion about it would pass by proving nothing. Cancelled with the
     * test, because its context is the background scope's job.
     */
    private fun TestScope.storeScope(): CoroutineScope =
        CoroutineScope(backgroundScope.coroutineContext + UnconfinedTestDispatcher(testScheduler))

    /** One runner, scripted. */
    private class FakeSource : ChangesSource {
        var set = ChangeSet()
        var setFails: Exception? = null
        var diffFails: Exception? = null
        var commitFilesFail: Exception? = null
        var commitFilesFor: MutableMap<String, List<ChangedFile>> = mutableMapOf()
        var diffs: MutableMap<String, FileDiffReply> = mutableMapOf()

        val changeSetCalls = mutableListOf<Boolean>()
        val diffCalls = mutableListOf<Pair<String, String>>()
        val commitFilesCalls = mutableListOf<String>()
        var markReadCalls = 0
        var refreshCountsCalls = 0
        val prompts = mutableListOf<Pair<String, String>>()

        /** Held open so a test can land an answer after the reader has moved on. */
        var gate: CompletableDeferred<Unit>? = null

        override suspend fun changeSet(workspace: String, fresh: Boolean): ChangeSet {
            changeSetCalls += fresh
            setFails?.let { throw it }
            return set
        }

        override suspend fun fileDiff(
            workspace: String,
            path: String,
            scope: String,
        ): FileDiffReply {
            diffCalls += path to scope
            gate?.await()
            diffFails?.let { throw it }
            return diffs[path] ?: FileDiffReply(path = path)
        }

        override suspend fun commitFiles(workspace: String, sha: String): List<ChangedFile> {
            commitFilesCalls += sha
            commitFilesFail?.let { throw it }
            return commitFilesFor[sha] ?: emptyList()
        }

        override suspend fun setBase(workspace: String, baseRef: String): ChangeSet = set

        override suspend fun markRead(workspace: String) {
            markReadCalls += 1
        }

        override suspend fun refreshCounts() {
            refreshCountsCalls += 1
        }

        override suspend fun agentPrompt(terminal: String, text: String) {
            prompts += terminal to text
        }
    }

    private fun file(path: String, plus: Int = 1) =
        ChangedFile(path = path, statusWord = "modified", insertions = plus)

    private fun diff(vararg lines: Pair<String, Int>) = FileDiffReply(
        hunks = listOf(
            DiffHunkReply(
                lines = lines.map { DiffLineReply(kind = it.first, newNumber = it.second, text = "x") }
            )
        )
    )

    // ---- reading ----

    @Test
    fun `a load fills the state and clears the error`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(branch = "feat/x", files = listOf(file("a.rs"), file("b.rs")))
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        assertEquals("feat/x", store.state.value.changeSet.branch)
        assertEquals(listOf("a.rs", "b.rs"), store.state.value.files.map { it.path })
        assertNull(store.state.value.error)
        assertFalse(store.state.value.loading)
        assertEquals(listOf(false), source.changeSetCalls)
    }

    /**
     * The view calls this on every appearance and a load throws away every diff it
     * holds, so calling it each time would empty the screen and refetch it
     * whenever somebody glanced at another chip and came back.
     */
    @Test
    fun `loadIfNeeded reads once and a refresh is explicit`() = runTest {
        val source = FakeSource()
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.loadIfNeeded()
        store.loadIfNeeded()
        assertEquals(1, source.changeSetCalls.size)
        store.load(fresh = true)
        assertEquals(listOf(false, true), source.changeSetCalls)
    }

    /**
     * **A failure is NOT an empty diff.** Saying so was a real bug on the Mac
     * once: a runner whose daemon predated this answered NOT_FOUND to every call
     * and the pane drew a worktree with no changes in it.
     */
    @Test
    fun `a failed read is a sentence, not a clean branch`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(branch = "feat/x", files = listOf(file("a.rs")))
            setFails = CoreException("method not found: changes.change_set")
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        assertTrue(store.state.value.files.isEmpty())
        assertEquals(
            "This runner’s Far Cooler is too old to review changes.",
            store.state.value.error?.sentence,
        )
        // The arm that names its own cause passes no transcript: a dump under a
        // sentence that already answers the question is noise.
        assertNull(store.state.value.error?.transcript)
    }

    @Test
    fun `a failure with no diagnosis carries the runner's own words`() = runTest {
        val source = FakeSource().apply { setFails = CoreException("ssh: channel closed") }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        assertEquals(
            "Couldn’t read this workspace. The request that reads it didn’t finish.",
            store.state.value.error?.sentence,
        )
        assertEquals("ssh: channel closed", store.state.value.error?.transcript)
    }

    // ---- one file at a time ----

    @Test
    fun `a file is read once and cached`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(files = listOf(file("a.rs")))
            diffs["a.rs"] = diff("context" to 1, "added" to 2)
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.ensure("a.rs")
        store.ensure("a.rs")
        assertEquals(listOf("a.rs" to "branch"), source.diffCalls)
        val lines = store.state.value.fileDiffs["a.rs"]
        assertEquals(2, lines?.size)
        assertEquals(DiffComputation.Kind.ADDED, lines?.get(1)?.kind)
        assertTrue(store.state.value.loadingFiles.isEmpty())
    }

    /**
     * `git diff` compares against something recorded and nothing is recorded for a
     * file only just written, so there is no round trip to spend.
     */
    @Test
    fun `an untracked file is never fetched`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(
                workingTree = WorkingTree(untracked = listOf("notes/new.md")),
            )
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.showUncommitted()
        store.ensure("notes/new.md")
        assertTrue(source.diffCalls.isEmpty())
    }

    @Test
    fun `a patch the daemon would not render says which nothing it is`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(files = listOf(file("logo.png")))
            diffs["logo.png"] = FileDiffReply(path = "logo.png", unsupported = "binary")
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.ensure("logo.png")
        assertEquals("Binary file", store.state.value.unsupported["logo.png"])
        assertEquals(emptyList<DiffComputation.Line>(), store.state.value.fileDiffs["logo.png"])
    }

    /**
     * Left unread rather than recorded as empty, so pulling to refresh tries it
     * again instead of showing a permanent blank.
     */
    @Test
    fun `a failed patch is unread rather than empty`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(files = listOf(file("a.rs")))
            diffFails = CoreException("boom")
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.ensure("a.rs")
        assertFalse(store.state.value.fileDiffs.containsKey("a.rs"))
        store.ensure("a.rs")
        assertEquals(2, source.diffCalls.size)
    }

    /**
     * One ahead, and only from the file that is actually open — so Next lands on a
     * patch instead of a spinner, and the chain stops at one because a prefetched
     * file is not expanded.
     */
    @Test
    fun `reading the open file reads the one after it and stops`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(files = listOf(file("a.rs"), file("b.rs"), file("c.rs")))
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.expand("a.rs")
        store.ensure("a.rs")
        assertEquals(listOf("a.rs", "b.rs"), source.diffCalls.map { it.first })
    }

    @Test
    fun `reading a file nobody opened reads nothing else`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(files = listOf(file("a.rs"), file("b.rs")))
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.ensure("a.rs")
        assertEquals(listOf("a.rs"), source.diffCalls.map { it.first })
    }

    // ---- the scope, and what a change of it forgets ----

    /**
     * The diffs on hand answer the OTHER question. Kept, they would show a
     * committed patch under an "Uncommitted" heading — or one commit's patch under
     * another commit's subject, which is the same bug wearing a better disguise.
     */
    @Test
    fun `changing the scope drops everything that answered the old one`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(
                files = listOf(file("a.rs")),
                workingTree = WorkingTree(unstaged = listOf("a.rs")),
            )
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.expand("a.rs")
        store.ensure("a.rs")
        val before = store.state.value.generation
        assertTrue(store.state.value.fileDiffs.containsKey("a.rs"))

        store.showUncommitted()

        val after = store.state.value
        assertEquals(DiffScope.Local, after.scope)
        assertTrue(after.fileDiffs.isEmpty())
        assertTrue(after.unsupported.isEmpty())
        assertNull(after.expandedFile)
        assertTrue(after.commitFiles.isEmpty())
        assertEquals(before + 1, after.generation)
    }

    /**
     * **The generation guard.** A read in flight when the reader moves on must not
     * be filed under what they moved to: every path answers with a well-formed
     * result, and a patch keyed on a path alone slots perfectly under a heading
     * that is now showing something else.
     */
    @Test
    fun `a patch that lands after the reader moved on is discarded`() = runTest {
        val gate = CompletableDeferred<Unit>()
        val source = FakeSource().apply {
            set = ChangeSet(
                files = listOf(file("a.rs")),
                workingTree = WorkingTree(unstaged = listOf("a.rs")),
            )
            diffs["a.rs"] = diff("added" to 1)
            this.gate = gate
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()

        storeScope().launch { store.ensure("a.rs") }
        assertEquals(1, source.diffCalls.size)

        // The reader switches comparison while the read is still crossing the link.
        store.showUncommitted()
        gate.complete(Unit)

        // The branch's patch for `a.rs` would have filed perfectly under the
        // uncommitted heading for the same path.
        assertFalse(store.state.value.fileDiffs.containsKey("a.rs"))
    }

    // ---- one commit at a time ----

    @Test
    fun `selecting a commit fetches its own file list`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(
                commits = listOf(ChangeCommit(sha = "aaa", subject = "one")),
                files = listOf(file("a.rs"), file("b.rs")),
            )
            commitFilesFor["aaa"] = listOf(file("a.rs"))
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.select("aaa")
        assertEquals(DiffScope.Commit("aaa"), store.state.value.scope)
        assertEquals(listOf("a.rs"), store.state.value.files.map { it.path })
        assertFalse(store.state.value.loading)
        // Asking for the same one again would spend a round trip to arrive back
        // where it already is.
        store.select("aaa")
        assertEquals(listOf("aaa"), source.commitFilesCalls)
    }

    /**
     * A commit that could not be read and a commit that changed nothing are two
     * different things, and only one of them is worth a warning triangle.
     */
    @Test
    fun `a commit that could not be read says so rather than looking empty`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(commits = listOf(ChangeCommit(sha = "aaa")))
            commitFilesFail = CoreException("unknown revision")
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.select("aaa")
        assertTrue(store.state.value.commitFiles.isEmpty())
        assertTrue(store.state.value.commitUnreadable)
    }

    @Test
    fun `going back to the branch costs no round trip`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(
                commits = listOf(ChangeCommit(sha = "aaa")),
                files = listOf(file("a.rs")),
            )
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.select("aaa")
        val calls = source.changeSetCalls.size
        store.showWholeBranch()
        assertEquals(listOf("a.rs"), store.state.value.files.map { it.path })
        assertEquals(calls, source.changeSetCalls.size)
    }

    // ---- what stays open ----

    /**
     * Not "collapse everything on load": a poll that picks up one new commit would
     * fold away the file somebody is mid-way through reading.
     */
    @Test
    fun `a refresh keeps the open file open unless it has left the diff`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(files = listOf(file("a.rs"), file("b.rs")))
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.expand("b.rs")
        source.set = ChangeSet(files = listOf(file("a.rs"), file("b.rs"), file("c.rs")))
        store.load()
        assertEquals("b.rs", store.state.value.expandedFile)

        source.set = ChangeSet(files = listOf(file("a.rs")))
        store.load()
        assertNull(store.state.value.expandedFile)
    }

    // ---- moving, and the jump that follows ----

    /**
     * Compose has no `scrollTo(id)`, so a jump names a row key and the model
     * resolves it against the list the view draws. See `Jump`.
     */
    @Test
    fun `opening a file raises a jump the view can turn into an index`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(files = listOf(file("a.rs"), file("Cargo.lock"), file("b.rs")))
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.showNextFile()
        val first = requireNotNull(store.state.value.jump)
        assertEquals("a.rs", first.key)
        assertEquals(1, store.state.value.indexOf(first.key))

        store.clearJump(first.serial)
        assertNull(store.state.value.jump)

        store.showNextFile()
        assertEquals("b.rs", store.state.value.jump?.key)
        // Third row, because the generated heading sits between b.rs and the
        // lockfile — an index that skipped it would land one file short.
        assertEquals(2, store.state.value.indexOf("b.rs"))
        store.showNextFile()
        assertEquals("Cargo.lock", store.state.value.jump?.key)
        assertEquals(4, store.state.value.indexOf("Cargo.lock"))
    }

    /**
     * Tapping the same file twice has to move the scroll both times, which a bare
     * key cannot express — and a clear that belongs to the previous jump must not
     * swallow the next one.
     */
    @Test
    fun `a second jump to the same file is a different jump`() = runTest {
        val source = FakeSource().apply { set = ChangeSet(files = listOf(file("a.rs"))) }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.expand("a.rs")
        val first = store.state.value.jump!!
        store.toggle("a.rs")
        store.expand("a.rs")
        val second = store.state.value.jump!!
        assertEquals(first.key, second.key)
        assertTrue(second.serial > first.serial)
        // The stale clear is a no-op.
        store.clearJump(first.serial)
        assertEquals(second, store.state.value.jump)
    }

    // ---- coming back ----

    private fun storageWith(position: ReviewPosition): InMemoryReviewStorage {
        val storage = InMemoryReviewStorage()
        ReviewBookmarks.write(storage, ref, position)
        return storage
    }

    @Test
    fun `a bookmark is offered rather than applied`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(files = listOf(file("a.rs"), file("b.rs")))
        }
        val storage = storageWith(ReviewPosition(scope = "branch", file = "b.rs", savedAt = 1))
        val store = ChangesStore(ref, source, storage, storeScope())
        store.load()
        assertEquals("b.rs", store.state.value.resume?.file)
        // Nothing has moved until it is taken.
        assertNull(store.state.value.expandedFile)

        store.applyResume()
        assertEquals("b.rs", store.state.value.expandedFile)
        assertNull(store.state.value.resume)
        assertNull(store.state.value.resumeNote)
    }

    /**
     * A pull to refresh is somebody asking about the branch they are already
     * reading; re-offering to take them somewhere else would be the app arguing
     * with them.
     */
    @Test
    fun `the offer is made once per run`() = runTest {
        val source = FakeSource().apply { set = ChangeSet(files = listOf(file("b.rs"))) }
        val storage = storageWith(ReviewPosition(file = "b.rs", savedAt = 1))
        val store = ChangesStore(ref, source, storage, storeScope())
        store.load()
        store.dismissResume()
        store.load(fresh = true)
        assertNull(store.state.value.resume)
    }

    /** Where everybody starts is not somewhere worth an interruption. */
    @Test
    fun `a bookmark at the top of the list is not offered`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(files = listOf(file("a.rs"), file("b.rs")))
        }
        val storage = storageWith(ReviewPosition(scope = "branch", topFile = "a.rs", savedAt = 1))
        val store = ChangesStore(ref, source, storage, storeScope())
        store.load()
        assertNull(store.state.value.resume)
    }

    /**
     * An amend or a rebase overnight is not an edge case on an agent-authored
     * branch. The rule is to land as close as possible and to SAY so.
     */
    @Test
    fun `a commit that was rebased away lands on the branch and says why`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(
                commits = listOf(ChangeCommit(sha = "aaa")),
                files = listOf(file("a.rs")),
            )
        }
        val storage = storageWith(ReviewPosition(scope = "gonegonegone", file = "a.rs", savedAt = 1))
        val store = ChangesStore(ref, source, storage, storeScope())
        store.load()
        store.applyResume()
        assertEquals(DiffScope.Branch, store.state.value.scope)
        assertEquals(ChangesRow.TOP_KEY, store.state.value.jump?.key)
        assertTrue(
            store.state.value.resumeNote!!.startsWith("That commit isn’t on the branch anymore")
        )
    }

    @Test
    fun `a file that left the diff lands at the top and names itself`() = runTest {
        val source = FakeSource().apply { set = ChangeSet(files = listOf(file("a.rs"))) }
        val storage = storageWith(
            ReviewPosition(scope = "branch", file = "crates/gone/away.rs", savedAt = 1)
        )
        val store = ChangesStore(ref, source, storage, storeScope())
        store.load()
        store.applyResume()
        assertEquals(ChangesRow.TOP_KEY, store.state.value.jump?.key)
        assertEquals(
            "away.rs isn’t in this diff anymore, so this is the top.",
            store.state.value.resumeNote,
        )
        assertNull(store.state.value.expandedFile)
    }

    @Test
    fun `a remembered top file is scrolled to without being opened`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(files = listOf(file("a.rs"), file("b.rs")))
        }
        val storage = storageWith(ReviewPosition(scope = "local", topFile = "b.rs", savedAt = 1))
        val store = ChangesStore(ref, source, storage, storeScope())
        store.load()
        store.applyResume()
        assertEquals(DiffScope.Local, store.state.value.scope)
        // `local` has no files here, so there is nothing to scroll to and nothing
        // is claimed. The scope still moved, which is the part that was saved.
        assertNull(store.state.value.expandedFile)
    }

    /**
     * The first rows realize themselves the moment the list draws and each reports
     * its visibility — so without this guard, arriving at the screen would
     * overwrite the very position the resume card is offering.
     */
    @Test
    fun `nothing is written down while an offer is on the table`() = runTest {
        val source = FakeSource().apply { set = ChangeSet(files = listOf(file("a.rs"), file("b.rs"))) }
        val storage = storageWith(ReviewPosition(scope = "branch", file = "b.rs", savedAt = 1))
        val store = ChangesStore(ref, source, storage, storeScope())
        store.load()
        store.noteTopFile("a.rs")
        assertEquals("b.rs", ReviewBookmarks.read(storage, ref)?.file)

        // Once it has been taken or declined, the next scroll follows the reader.
        store.dismissResume()
        store.noteTopFile("b.rs")
        assertEquals("b.rs", ReviewBookmarks.read(storage, ref)?.topFile)
        assertNull(ReviewBookmarks.read(storage, ref)?.file)
    }

    @Test
    fun `the bookmark follows the scope and the open file`() = runTest {
        val storage = InMemoryReviewStorage()
        val source = FakeSource().apply {
            set = ChangeSet(
                files = listOf(file("a.rs")),
                workingTree = WorkingTree(unstaged = listOf("a.rs")),
            )
        }
        val store = ChangesStore(ref, source, storage, storeScope())
        store.load()
        store.expand("a.rs")
        assertEquals("a.rs", ReviewBookmarks.read(storage, ref)?.file)
        store.showUncommitted()
        val saved = ReviewBookmarks.read(storage, ref)
        assertEquals("local", saved?.scope)
        // The open file went with the scope, so the bookmark stops claiming one.
        assertNull(saved?.file)
    }

    // ---- the two writes ----

    /**
     * **Marking read must not reload the diff.** `92058f4` found this exact shape
     * on the Mac: the one gesture meaning "I have finished reading this" dropped
     * every fetched patch and closed every fold. Marking read changes the inbox,
     * so the inbox is what gets refreshed.
     */
    @Test
    fun `marking read refreshes the counts and keeps every patch on hand`() = runTest {
        val source = FakeSource().apply {
            set = ChangeSet(files = listOf(file("a.rs")))
            diffs["a.rs"] = diff("added" to 1)
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.load()
        store.expand("a.rs")
        store.ensure("a.rs")
        val reads = source.changeSetCalls.size

        store.markRead()

        assertEquals(1, source.markReadCalls)
        assertEquals(1, source.refreshCountsCalls)
        assertEquals("the diff must not be re-read", reads, source.changeSetCalls.size)
        assertTrue(store.state.value.fileDiffs.containsKey("a.rs"))
        assertEquals("a.rs", store.state.value.expandedFile)
    }

    @Test
    fun `pinning a base replaces the change set and everything read against the old one`() =
        runTest {
            val source = FakeSource().apply {
                set = ChangeSet(baseSource = "guessed", files = listOf(file("a.rs")))
                diffs["a.rs"] = diff("added" to 1)
            }
            val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
            store.load()
            store.ensure("a.rs")
            assertTrue(store.state.value.changeSet.baseIsGuessed)

            source.set = ChangeSet(baseSource = "recorded", files = listOf(file("a.rs")))
            store.setBase("origin/main")

            assertFalse(store.state.value.changeSet.baseIsGuessed)
            assertTrue(store.state.value.fileDiffs.isEmpty())
            assertFalse(store.state.value.loading)
        }

    // ---- the outbox's one runner-facing edge ----

    @Test
    fun `a batch of notes goes out as an agent prompt`() = runTest {
        val source = FakeSource()
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.comments.write(ReviewAnchor(file = "a.rs", firstLine = 3), "rename this")
        store.comments.send(
            com.farcooler.model.ReviewAgentTarget(id = "t1", name = "claude"),
            "feat/x",
        )
        assertEquals(1, source.prompts.size)
        assertEquals("t1", source.prompts[0].first)
        assertTrue(source.prompts[0].second.contains("`a.rs`, around line 3"))
        assertTrue(store.comments.state.value.pending.isEmpty())
    }

    @Test
    fun `a dropped link keeps the notes and says which failure it was`() = runTest {
        val source = object : ChangesSource by FakeSource() {
            override suspend fun agentPrompt(terminal: String, text: String) {
                throw DisconnectedException("the session is gone")
            }
        }
        val store = ChangesStore(ref, source, InMemoryReviewStorage(), storeScope())
        store.comments.write(ReviewAnchor(file = "a.rs"), "rename this")
        store.comments.send(
            com.farcooler.model.ReviewAgentTarget(id = "t1", name = "claude"),
            "feat/x",
        )
        assertEquals(1, store.comments.state.value.pending.size)
        assertTrue(
            store.comments.state.value.failure!!.sentence
                .startsWith("The connection to this runner dropped")
        )
    }
}
