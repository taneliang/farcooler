package com.farcooler.model

import com.farcooler.data.InMemoryReviewStorage
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The two things a review leaves behind, and the derivation that feeds one.
 *
 * Both outlive the process, which is the whole reason they are written down at
 * all: the situation this surface is built for is a dozen ninety-second windows
 * across an hour, on a phone Android is free to kill between any two of them.
 * Neither can be exercised on a device from here — there is no emulator for this
 * phase — so the storage is a seam and these are the proof.
 */
class ReviewTest {
    private val ref = ReviewRef("host-a", "8f14e45f-ce5b-4a5e-9c2b-000000000001")

    // ---- the bookmark ----

    @Test
    fun `a position round trips through storage`() {
        val storage = InMemoryReviewStorage()
        val saved = ReviewPosition(
            scope = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            file = "crates/daemon/src/review_ops.rs",
            topFile = "crates/core/src/feed.rs",
            savedAt = 1_755_900_000,
        )
        ReviewBookmarks.write(storage, ref, saved)
        assertEquals(saved, ReviewBookmarks.read(storage, ref))
        ReviewBookmarks.forget(storage, ref)
        assertNull(ReviewBookmarks.read(storage, ref))
    }

    /**
     * **The collision `df87410` had to solve for the front door.** Workspace ids
     * are minted per daemon and this app connects to every runner at once, so two
     * runners can hold a worktree with the same id — and a bookmark keyed on the
     * workspace alone would resume "where you were" in the wrong worktree.
     */
    @Test
    fun `two runners holding the same workspace id keep separate bookmarks`() {
        val storage = InMemoryReviewStorage()
        val other = ReviewRef("host-b", ref.workspaceId)
        ReviewBookmarks.write(storage, ref, ReviewPosition(file = "a.rs"))
        ReviewBookmarks.write(storage, other, ReviewPosition(file = "b.rs"))
        assertEquals("a.rs", ReviewBookmarks.read(storage, ref)?.file)
        assertEquals("b.rs", ReviewBookmarks.read(storage, other)?.file)
        assertEquals("host-a/${ref.workspaceId}", ref.key)
    }

    @Test
    fun `unreadable stored bytes are no bookmark rather than a crash`() {
        val storage = InMemoryReviewStorage()
        storage.write("changes.position.${ref.key}", "{not json")
        assertNull(ReviewBookmarks.read(storage, ref))
    }

    /**
     * A record written by a newer build. One unknown key must not cost somebody
     * their place.
     */
    @Test
    fun `a key this build does not know does not cost the position`() {
        val storage = InMemoryReviewStorage()
        storage.write(
            "changes.position.${ref.key}",
            """{"scope":"local","file":"a.rs","hunk":3,"savedAt":7}""",
        )
        val position = ReviewBookmarks.read(storage, ref)
        assertEquals("local", position?.scope)
        assertEquals("a.rs", position?.file)
    }

    /**
     * Where everybody starts is not somewhere worth being offered a trip back to.
     */
    @Test
    fun `the top of the branch is not somewhere`() {
        assertFalse(ReviewPosition().isSomewhere)
        assertTrue(ReviewPosition(file = "a.rs").isSomewhere)
        assertTrue(ReviewPosition(topFile = "a.rs").isSomewhere)
        assertTrue(ReviewPosition(scope = "local").isSomewhere)
        assertTrue(ReviewPosition(scope = "deadbeef").isSomewhere)
    }

    /** The protocol's own rule, read in reverse: anything that is not a name is a sha. */
    @Test
    fun `a saved scope names a sha or it names a scope`() {
        assertNull(ReviewPosition.sha("branch"))
        assertNull(ReviewPosition.sha("local"))
        assertNull(ReviewPosition.sha("staged"))
        assertNull(ReviewPosition.sha("unstaged"))
        assertNull(ReviewPosition.sha(""))
        assertEquals("deadbeef", ReviewPosition.sha("deadbeef"))
    }

    // ---- the anchor Compose has to derive for itself ----

    /**
     * iOS gets this from `.onScrollVisibilityChange` per heading. Compose has no
     * such modifier, so the same answer comes out of `layoutInfo.visibleItemsInfo`
     * — which arrives in index order, so "the reading-order first one on screen"
     * is the first element that passes the threshold and no set has to be kept.
     */
    @Test
    fun `the topmost visible file is the first one far enough into the viewport`() {
        val rows = listOf(
            ReviewScroll.VisibleRow(ChangesRow.TOP_KEY, -400, 300),
            ReviewScroll.VisibleRow("a.rs", -100, 200),
            ReviewScroll.VisibleRow("b.rs", 100, 200),
        )
        assertEquals("a.rs", ReviewScroll.topVisible(rows, 0, 800) { it.startsWith("a") || it.startsWith("b") })
    }

    /**
     * The summary block is at the top of every list and is not a place in the
     * diff. Returning it would make every bookmark say "the top", and the resume
     * offer would never have anything to restore.
     */
    @Test
    fun `a row that is not a file is skipped rather than returned`() {
        val rows = listOf(
            ReviewScroll.VisibleRow(ChangesRow.TOP_KEY, 0, 300),
            ReviewScroll.VisibleRow(ChangesRow.GENERATED_KEY, 300, 40),
            ReviewScroll.VisibleRow("Cargo.lock", 340, 200),
        )
        assertEquals(
            "Cargo.lock",
            ReviewScroll.topVisible(rows, 0, 800) { it == "Cargo.lock" },
        )
    }

    /**
     * A row scrolled almost entirely past the top is not the file being read. The
     * threshold is tiny on purpose — a heading with a sliver showing is still the
     * file whose hunks fill the screen — so this has to be a real sliver.
     */
    @Test
    fun `a row barely off the top is passed over`() {
        val rows = listOf(
            // 4 pixels of 200 showing, under the 5% threshold.
            ReviewScroll.VisibleRow("a.rs", -196, 200),
            ReviewScroll.VisibleRow("b.rs", 4, 200),
        )
        assertEquals("b.rs", ReviewScroll.topVisible(rows, 0, 800) { true })
        // 10 of 200 is above it, and that row is the answer.
        val sliver = listOf(
            ReviewScroll.VisibleRow("a.rs", -190, 200),
            ReviewScroll.VisibleRow("b.rs", 10, 200),
        )
        assertEquals("a.rs", ReviewScroll.topVisible(sliver, 0, 800) { true })
    }

    @Test
    fun `nothing on screen is no answer rather than a wrong one`() {
        assertNull(ReviewScroll.topVisible(emptyList(), 0, 800) { true })
        assertNull(
            ReviewScroll.topVisible(
                listOf(ReviewScroll.VisibleRow("a.rs", 0, 0)), 0, 800
            ) { true }
        )
    }

    /**
     * The other half of the sticky file name, added by phase 5b.
     *
     * A file's heading is drawn at the very top of its own row, so "has its
     * heading gone" is the same question as "is its top above the viewport". The
     * bar is drawn only when it is, or the same file's name would be on the
     * screen twice.
     */
    @Test
    fun `the pinned name appears only once the card's own heading has gone`() {
        val rows = listOf(
            ReviewScroll.VisibleRow("a.rs", -900, 2_000),
            ReviewScroll.VisibleRow("b.rs", 1_100, 400),
        )
        assertTrue(ReviewScroll.scrolledPastHeading(rows, "a.rs", 0))
        assertFalse(ReviewScroll.scrolledPastHeading(rows, "b.rs", 0))
    }

    /**
     * A row the list is not currently showing has no offset to compare. False,
     * rather than a guess, because the alternative is a bar naming a file that
     * is nowhere on the screen.
     */
    @Test
    fun `a row that is not laid out is not scrolled past`() {
        assertFalse(ReviewScroll.scrolledPastHeading(emptyList(), "a.rs", 0))
    }

    // ---- what a comment is attached to ----

    @Test
    fun `an anchor reads one way in the app and another to the agent`() {
        val whole = ReviewAnchor(file = "push.ts")
        assertEquals("the whole file", whole.placeDescription)
        assertEquals("`push.ts`", whole.promptDescription)

        val hunk = ReviewAnchor(file = "push.ts", firstLine = 120, lastLine = 148)
        assertEquals("lines 120-148", hunk.placeDescription)
        assertEquals("`push.ts`, around lines 120-148", hunk.promptDescription)

        val one = ReviewAnchor(file = "push.ts", firstLine = 120, lastLine = 120)
        assertEquals("line 120", one.placeDescription)

        val onCommit = ReviewAnchor(
            file = "push.ts", commit = "aaaaaaaabbbbbbbb", firstLine = 7
        )
        assertEquals("`push.ts` (commit aaaaaaaa), around line 7", onCommit.promptDescription)
    }

    @Test
    fun `a quote is trimmed, capped, and never empty`() {
        assertNull(ReviewAnchor.quoting("   \n  "))
        assertEquals("let x = 1", ReviewAnchor.quoting("   let x = 1  "))
        val long = "x".repeat(200)
        val quoted = requireNotNull(ReviewAnchor.quoting(long))
        assertEquals(ReviewAnchor.QUOTE_LIMIT + 1, quoted.length)
        assertTrue(quoted.endsWith("…"))
    }

    // ---- the outbox ----

    private fun queue(
        storage: InMemoryReviewStorage,
        deliver: suspend (ReviewAgentTarget, String) -> Trouble? = { _, _ -> null },
    ) = ReviewCommentQueue(ref, storage, deliver, now = { 1_755_900_000 })

    private val target = ReviewAgentTarget(id = "t1", name = "claude 2")

    /**
     * **Persisted on every write**, because the app holding a note can be killed
     * between the window it was typed in and the window it would be sent in.
     */
    @Test
    fun `a note is on disk the moment it is written`() {
        val storage = InMemoryReviewStorage()
        val q = queue(storage)
        assertEquals(0, storage.writes)
        q.write(ReviewAnchor(file = "a.rs", firstLine = 3), "handle 429 as well")
        assertEquals(1, storage.writes)

        // A new queue over the same storage is what a process death produces.
        val revived = queue(storage)
        assertEquals(1, revived.state.value.pending.size)
        assertEquals("handle 429 as well", revived.state.value.pending[0].text)
        assertEquals(3, revived.state.value.pending[0].anchor.firstLine)
        // Reading it back must not write it back.
        assertEquals(1, storage.writes)
    }

    @Test
    fun `two runners holding the same workspace id keep separate outboxes`() {
        val storage = InMemoryReviewStorage()
        queue(storage).write(ReviewAnchor(file = "a.rs"), "one")
        val other = ReviewCommentQueue(
            ReviewRef("host-b", ref.workspaceId), storage, { _, _ -> null }
        )
        assertTrue(other.state.value.pending.isEmpty())
    }

    @Test
    fun `an empty note is not a note`() {
        val storage = InMemoryReviewStorage()
        val q = queue(storage)
        q.write(ReviewAnchor(file = "a.rs"), "   ")
        assertTrue(q.state.value.pending.isEmpty())
        assertEquals(0, storage.writes)
    }

    /**
     * Numbered and grouped in the order they were written, which is reading order
     * — the order the reader went through the diff in, and therefore the order in
     * which the notes make sense to each other.
     */
    @Test
    fun `the batch is one message in reading order`() {
        val q = queue(InMemoryReviewStorage())
        q.write(ReviewAnchor(file = "push.ts", firstLine = 120, lastLine = 148, quote = "await fetch(url)"), "handle 429 as well")
        q.write(ReviewAnchor(file = "retry.ts"), "this belongs next to the backoff")
        assertEquals(
            """
            Review notes on `feat/x` from Far Cooler (2):

            1. In `push.ts`, around lines 120-148:
               > await fetch(url)
               handle 429 as well

            2. In `retry.ts`:
               this belongs next to the backoff
            """.trimIndent(),
            q.message("feat/x"),
        )
    }

    @Test
    fun `one note says note and a branchless batch says so too`() {
        val q = queue(InMemoryReviewStorage())
        q.write(ReviewAnchor(file = "a.rs"), "rename this")
        assertTrue(q.message("").startsWith("Review note from Far Cooler (1):"))
        assertTrue(q.message("main").startsWith("Review note on `main` from Far Cooler (1):"))
    }

    @Test
    fun `a sent batch leaves a receipt and empties the queue`() = runTest {
        val storage = InMemoryReviewStorage()
        var handed: String? = null
        val q = queue(storage) { _, text -> handed = text; null }
        q.write(ReviewAnchor(file = "a.rs"), "rename this")
        q.send(target, "feat/x")

        val state = q.state.value
        assertTrue(state.pending.isEmpty())
        assertEquals(1, state.sent.size)
        assertEquals("claude 2", state.sent[0].agentName)
        assertEquals(1, state.sent[0].count)
        assertEquals(1_755_900_000L, state.sent[0].sentAt)
        assertEquals(handed, state.sent[0].text)
        assertNull(state.failure)
        assertFalse(state.sending)
        // And a process death after a send finds an empty queue and the receipt.
        val revived = queue(storage)
        assertTrue(revived.state.value.pending.isEmpty())
        assertEquals(1, revived.state.value.sent.size)
    }

    /**
     * **Nothing is ever retried automatically.** `request_no_wait` means a failure
     * here is "this client did not get an answer", which is not the same as "the
     * agent did not get the prompt", and only a person can weigh the difference.
     */
    @Test
    fun `a failed send keeps the notes and does not try again`() = runTest {
        val storage = InMemoryReviewStorage()
        var attempts = 0
        val q = queue(storage) { _, _ ->
            attempts += 1
            Trouble("Couldn’t send these. They’re still here.", "empty session slot")
        }
        q.write(ReviewAnchor(file = "a.rs"), "rename this")
        q.send(target, "feat/x")

        assertEquals(1, attempts)
        assertEquals(1, q.state.value.pending.size)
        assertTrue(q.state.value.sent.isEmpty())
        assertEquals("empty session slot", q.state.value.failure?.transcript)
        assertFalse(q.state.value.sending)

        // The notes survive the process, and still nothing has been re-sent.
        val revived = queue(storage)
        assertEquals(1, revived.state.value.pending.size)
        // A failure is not restored: it would be a warning about a request
        // nobody made this session.
        assertNull(revived.state.value.failure)
        assertEquals(1, attempts)
    }

    @Test
    fun `a new note clears the failure it has been read past`() = runTest {
        val q = queue(InMemoryReviewStorage()) { _, _ -> Trouble("no") }
        q.write(ReviewAnchor(file = "a.rs"), "one")
        q.send(target, "x")
        assertNotNull(q.state.value.failure)
        q.write(ReviewAnchor(file = "b.rs"), "two")
        assertNull(q.state.value.failure)
    }

    @Test
    fun `an empty queue sends nothing`() = runTest {
        var attempts = 0
        val q = queue(InMemoryReviewStorage()) { _, _ -> attempts += 1; null }
        q.send(target, "x")
        assertEquals(0, attempts)
    }

    /** A receipt and not a history: the question is "what did I just send". */
    @Test
    fun `only the last few receipts are kept`() = runTest {
        val q = queue(InMemoryReviewStorage())
        repeat(7) { index ->
            q.write(ReviewAnchor(file = "$index.rs"), "note $index")
            q.send(target, "x")
        }
        assertEquals(5, q.state.value.sent.size)
        // Newest first.
        assertTrue(q.state.value.sent[0].text.contains("note 6"))
    }

    @Test
    fun `a note can be taken back before it goes`() {
        val q = queue(InMemoryReviewStorage())
        q.write(ReviewAnchor(file = "a.rs"), "one")
        q.write(ReviewAnchor(file = "b.rs"), "two")
        q.remove(q.state.value.pending[0])
        assertEquals(listOf("two"), q.state.value.pending.map { it.text })
    }

    // ---- where a note can be sent ----

    /**
     * Both words for "an agent is in here", and one exclusion. A claude flipped
     * back to its raw terminal is still an agent holding an ACP session; a
     * `changes` pane is the diff of the thing being reviewed and could never
     * receive a note.
     */
    @Test
    fun `every agent pane is a target and the diff pane is not`() {
        val workspace = Workspace(
            id = "w",
            terminals = listOf(
                Terminal(id = "1", preset = "claude", paneMode = "agent", chatCapable = true),
                Terminal(id = "2", preset = "claude", paneMode = "terminal", chatCapable = true),
                Terminal(id = "3", preset = "zsh", paneMode = "terminal"),
                Terminal(id = "4", preset = "changes", paneMode = "changes", chatCapable = true),
            ),
        )
        val targets = workspace.reviewAgentTargets()
        assertEquals(listOf("1", "2"), targets.map { it.id })
        // Numbered, because two identical `claude`s are genuinely alike.
        assertEquals(listOf("claude 1", "claude 2"), targets.map { it.name })
        assertEquals(listOf(true, false), targets.map { it.showsChat })
    }
}
