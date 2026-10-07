package com.farcooler.net

import com.farcooler.core.CoreException
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The loop that pages, follows, and pages again (AgentKit's `AgentRowStore`). */
class AgentRowStoreTest {
    private val running = TestScope()
    private val source = FakeRowSource()
    private fun store(retryMs: Long = 1) = AgentRowStore(running.scope, retryDelayMs = retryMs, followWaitMs = 1)

    @After
    fun tearDown() = running.close()

    @Test
    fun `it pages, then follows by revision and applies the diff in place`() {
        val store = store()
        source.answerPage()
        store.start(source)
        eventually("the page") { store.shown.value.rows.size == 10 }
        eventually("following") { store.shown.value.phase == AgentRowStore.Phase.Live && source.followCalls.get() >= 1 }

        val ask = store.shown.value.rows.first { it.id == "ask:1" }
        source.answerFollow()
        eventually("the follow's rows") { store.shown.value.rows.any { it.id == "prose:2" } }
        val ids = store.shown.value.rows.map { it.id }
        assertFalse("queued:1" in ids)
        // What did not change is the object the list already drew.
        assertTrue(ask === store.shown.value.rows.first { it.id == "ask:1" })
        assertEquals(1, source.pageCalls.get())
    }

    @Test
    fun `a reset pages again`() {
        val store = store()
        source.answerPage()
        source.follows.trySend(
            Result.success(
                kotlinx.serialization.json.buildJsonObject {
                    put("epoch", kotlinx.serialization.json.JsonPrimitive(9))
                    put("rev", kotlinx.serialization.json.JsonPrimitive(1))
                    put("reset", kotlinx.serialization.json.JsonPrimitive(true))
                },
            ),
        )
        source.answerPage()
        store.start(source)
        eventually("a second page") { source.pageCalls.get() == 2 }
    }

    @Test
    fun `a failed call says trouble, keeps the rows, and pages again`() {
        val store = store()
        source.answerPage()
        store.start(source)
        eventually("live") { store.shown.value.phase == AgentRowStore.Phase.Live }
        source.failFollow(CoreException("The runner took too long to answer."))
        eventually("trouble") { store.shown.value.phase is AgentRowStore.Phase.Trouble }
        // The rows held stay, stale: a view says so over them.
        assertEquals(10, store.shown.value.rows.size)
        assertTrue(store.shown.value.isStale)
        source.answerPage()
        eventually("recovered") { store.shown.value.phase == AgentRowStore.Phase.Live }
        assertEquals(2, source.pageCalls.get())
        assertFalse(store.shown.value.isStale)
    }

    @Test
    fun `a runner that doesn't serve rows ends the loop and says so`() {
        val store = store()
        source.failPage(AgentRowsUnavailable())
        store.start(source)
        eventually("unavailable") { store.shown.value.phase == AgentRowStore.Phase.Unavailable }
        eventually("the loop to end") { !store.isFollowing }
    }

    @Test
    fun `stopping ends the loop and starting again follows from the cursor rather than paging`() {
        val store = store()
        source.answerPage()
        store.start(source)
        eventually("live") { store.shown.value.phase == AgentRowStore.Phase.Live && source.followCalls.get() >= 1 }
        store.stop()
        assertFalse(store.isFollowing)
        // Held rows are drawn as held, not as live, until the runner answers.
        store.start(source)
        assertEquals(AgentRowStore.Phase.Cached, store.shown.value.phase)
        source.answerNothing()
        eventually("live again") { store.shown.value.phase == AgentRowStore.Phase.Live }
        assertEquals(1, source.pageCalls.get())
    }

    private fun pageOf(epoch: Long, more: Boolean, vararg ords: Long) = kotlinx.serialization.json.buildJsonObject {
        put("epoch", kotlinx.serialization.json.JsonPrimitive(epoch))
        put("rev", kotlinx.serialization.json.JsonPrimitive(9))
        put("moreBefore", kotlinx.serialization.json.JsonPrimitive(more))
        put(
            "rows",
            kotlinx.serialization.json.JsonArray(
                ords.map { ord ->
                    kotlinx.serialization.json.buildJsonObject {
                        put("id", kotlinx.serialization.json.JsonPrimitive("prose:$ord"))
                        put("ord", kotlinx.serialization.json.JsonPrimitive(ord))
                        put("rev", kotlinx.serialization.json.JsonPrimitive(ord))
                        put("kind", kotlinx.serialization.json.buildJsonObject {
                            put("Prose", kotlinx.serialization.json.buildJsonObject {
                                put("text", kotlinx.serialization.json.JsonPrimitive("row $ord"))
                            })
                        })
                    }
                },
            ),
        )
    }

    @Test
    fun `older rows page in above, once at a time`() {
        val store = store()
        source.answerPage(pageOf(4, true, 5, 6))
        store.start(source)
        eventually("the page") { store.shown.value.rows.size == 2 && store.shown.value.moreBefore }
        source.answerPage(pageOf(4, false, 3, 4, 5))
        store.loadOlder(source)
        // A second ask while the first is out is not a second call.
        store.loadOlder(source)
        eventually("the older page") { store.shown.value.rows.size == 4 && !store.shown.value.loadingOlder }
        assertEquals(listOf("prose:3", "prose:4", "prose:5", "prose:6"), store.shown.value.rows.map { it.id })
        assertFalse(store.shown.value.moreBefore)
        assertEquals(1, source.olderCalls.get())
        // Nothing left to load: asking again is not a call.
        store.loadOlder(source)
        assertEquals(1, source.olderCalls.get())
    }

    @Test
    fun `a failed older page is counted so the view can ask again, and a good one clears the count`() {
        val store = store()
        source.answerPage(pageOf(4, true, 5, 6))
        store.start(source)
        eventually("the page") { store.shown.value.moreBefore }
        source.failPage(CoreException("The runner took too long to answer."))
        store.loadOlder(source)
        eventually("one failure") { store.shown.value.olderFailures == 1 && !store.shown.value.loadingOlder }
        // Still more to load: the view's spinner stays, and asks again.
        assertTrue(store.shown.value.moreBefore)
        source.failPage(CoreException("again"))
        store.loadOlder(source)
        eventually("two failures") { store.shown.value.olderFailures == 2 && !store.shown.value.loadingOlder }
        source.answerPage(pageOf(4, false, 3, 4, 5))
        store.loadOlder(source)
        eventually("cleared") { store.shown.value.olderFailures == 0 && store.shown.value.rows.size == 4 }
    }
}
