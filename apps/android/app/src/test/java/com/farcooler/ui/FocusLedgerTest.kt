package com.farcooler.ui

import com.farcooler.data.InMemoryReviewStorage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The tab chosen in each worktree, across a relaunch (ov-233), at the points
 * that save and load it: each test goes red when its save, load or prune is removed.
 */
class FocusLedgerTest {
    /** One process: its saved-state copy, and the app's preferences that outlive it. */
    private class Process(val kept: InMemoryReviewStorage, var saved: String? = null) {
        val ledger = FocusLedger({ saved }, { saved = it }, kept)
    }

    private fun term(id: String) = Pane.Terminal(id)

    @Test
    fun `a tab chosen in one launch is the tab the next launch opens on`() {
        val kept = InMemoryReviewStorage()
        val first = Process(kept)
        first.ledger.record("h/w1", Pane.Changes, chosen = true)
        first.ledger.record("h/w2", term("t1"), chosen = true)
        // The relaunch: the saved-state copy is gone, the preferences are not.
        val second = Process(kept)
        second.ledger.restore()
        assertEquals(Pane.Changes, second.ledger["h/w1"]?.pane)
        assertEquals(term("t1"), second.ledger["h/w2"]?.pane)
    }

    @Test
    fun `a pane somebody was only sent to is not kept`() {
        val kept = InMemoryReviewStorage()
        Process(kept).ledger.record("h/w1", Pane.Changes, chosen = false)
        val next = Process(kept)
        next.ledger.restore()
        assertNull(next.ledger["h/w1"])
    }

    @Test
    fun `a process death restores from its own copy first`() {
        val kept = InMemoryReviewStorage()
        val first = Process(kept)
        first.ledger.record("h/w1", Pane.Changes, chosen = true)
        val second = Process(kept, saved = Backstack.encodeFocus(mapOf("h/w1" to Focus(term("t9"), chosen = true))))
        second.ledger.restore()
        assertEquals(term("t9"), second.ledger["h/w1"]?.pane)
    }

    @Test
    fun `two runners reconnecting at once do not cost the first its agent tabs`() {
        val kept = InMemoryReviewStorage()
        val p = Process(kept)
        p.ledger.record("a/w1", term("t1"), chosen = true)
        p.ledger.record("b/w2", term("t2"), chosen = true)
        // B's fleet has been read; A is connected but its fleet hasn't arrived, so it has no terminals yet.
        p.ledger.prune(
            fleetRead = { it == "b" },
            hasWorktree = { host, _ -> host == "b" },
            hasTerminal = { host, _, _ -> host == "b" },
        )
        assertEquals(term("t1"), p.ledger["a/w1"]?.pane)
        val next = Process(kept)
        next.ledger.restore()
        assertEquals(term("t1"), next.ledger["a/w1"]?.pane)
    }

    @Test
    fun `a read fleet forgets a gone worktree and a gone agent, and keeps Changes where the worktree is`() {
        val kept = InMemoryReviewStorage()
        val p = Process(kept)
        p.ledger.record("a/gone", Pane.Changes, chosen = true)
        p.ledger.record("a/w1", Pane.Changes, chosen = true)
        p.ledger.record("a/w1b", term("dead"), chosen = true)
        p.ledger.prune(
            fleetRead = { true },
            hasWorktree = { _, worktree -> worktree != "gone" },
            hasTerminal = { _, _, _ -> false },
        )
        assertNull(p.ledger["a/gone"])
        assertNull(p.ledger["a/w1b"])
        assertEquals(Pane.Changes, p.ledger["a/w1"]?.pane)
        val next = Process(kept)
        next.ledger.restore()
        assertEquals(setOf("a/w1"), next.ledger.focus.value.keys)
        assertTrue(next.kept.read(Backstack.FOCUS_KEY)!!.contains("a/w1"))
    }

    /**
     * `AppModel` needs an `Application`, so a JVM test can't build it; these read
     * its source for the three calls that make the ledger the app's. Each goes
     * red when its call is removed, which is the restore going quietly dead.
     */
    private fun appModel(): String {
        var dir: java.io.File? = java.io.File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (dir != null) {
            val f = java.io.File(dir, "app/src/main/java/com/farcooler/ui/AppModel.kt")
            if (f.isFile) return f.readText()
            dir = dir.parentFile
        }
        throw AssertionError("no AppModel.kt above ${System.getProperty("user.dir")}")
    }

    @Test
    fun `the model restores the ledger at launch, records into it and prunes it`() {
        val source = appModel()
        assertTrue(source.contains("focusLedger.restore()"))
        assertTrue(source.substringAfter("private fun record(").substringBefore("keepDestination()").contains("focusLedger.record("))
        assertTrue(source.substringAfter("private fun settle()").substringBefore("\n    }\n").contains("focusLedger.prune("))
    }
}
