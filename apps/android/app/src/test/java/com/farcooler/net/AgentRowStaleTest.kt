package com.farcooler.net

import kotlinx.serialization.json.JsonObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

/**
 * ov-612: a stopped loop's follow that still answers changes nothing, as in
 * AgentKit's `AgentRowResyncTests`.
 */
class AgentRowStaleTest {
    private val running = TestScope()
    private val source = FakeRowSource()

    /**
     * The client core's call as it can be: once asked, it answers even after its
     * caller is cancelled. A plain `suspendCoroutine` is not cancellable, so its
     * answer always reaches the line after it, whatever kotlinx does on return.
     */
    private val held = java.util.concurrent.atomic.AtomicReference<kotlin.coroutines.Continuation<JsonObject>?>()
    private val uncancellable = object : AgentRowSource {
        override suspend fun page(before: Long?, limit: Int): JsonObject = source.page(before, limit)

        override suspend fun follow(epoch: Long, afterRev: Long, waitMs: Int): JsonObject {
            source.followCalls.incrementAndGet()
            return kotlin.coroutines.suspendCoroutine { held.set(it) }
        }
    }

    @After
    fun tearDown() = running.close()

    @Test
    fun `a follow that answers after the stop changes neither the ledger nor the view`() {
        val store = AgentRowStore(running.scope, retryDelayMs = 1, followWaitMs = 1)
        source.answerPage()
        store.start(uncancellable)
        eventually("the held follow") { store.shown.value.phase == AgentRowStore.Phase.Live && held.get() != null }
        val cursor = store.ledger.cursor
        val shown = store.shown.value.rows.map { it.id }

        store.stop()
        held.getAndSet(null)!!.resumeWith(Result.success(com.farcooler.model.AgentRowFixture.follow))
        // Either the ledger refuses the answer or (before the fix) takes it.
        eventually("the late answer handled") { store.ledger.refused > 0 || store.ledger.cursor != cursor }
        assertEquals("the stopped loop moved the cursor", cursor, store.ledger.cursor)
        assertFalse("prose:2" in store.ledger.held().map { it.id })
        assertEquals(shown, store.shown.value.rows.map { it.id })
    }
}
