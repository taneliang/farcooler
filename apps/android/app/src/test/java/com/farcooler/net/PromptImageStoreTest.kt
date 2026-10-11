package com.farcooler.net

import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** A prompt's images, fetched once each through `agent.image` (the Mac's `PromptImageStore`, ov-454, ov-458). */
class PromptImageStoreTest {
    private val asked = mutableListOf<String>()
    private val fetches = AtomicInteger()

    private fun store(source: PromptImageSource? = PromptImageSource { _, row, index ->
        fetches.incrementAndGet()
        asked.add("$row#$index")
        byteArrayOf(index.toByte())
    }) = PromptImageStore("t1").also { it.source = source }

    @Test
    fun `an image is fetched once, and shown with its bytes`() = runBlocking {
        val store = store()
        assertTrue(store.state("turn:0", 1) is PromptImageStore.State.Loading)
        store.load("turn:0", 1)
        store.load("turn:0", 1)
        assertEquals(1, fetches.get())
        assertEquals(listOf(1.toByte()), (store.state("turn:0", 1) as PromptImageStore.State.Shown).bytes.toList())
        // Another index of the same row is another image.
        store.load("turn:0", 0)
        assertEquals(listOf("turn:0#1", "turn:0#0"), asked)
    }

    @Test
    fun `a failed fetch says so, and is tried again when the row is shown anew`() = runBlocking {
        var fail = true
        val store = store(PromptImageSource { _, _, _ -> if (fail) throw IllegalStateException("gone") else byteArrayOf(7) })
        store.load("turn:0", 0)
        assertTrue(store.state("turn:0", 0) is PromptImageStore.State.Failed)
        fail = false
        store.load("turn:0", 0)
        assertTrue(store.state("turn:0", 0) is PromptImageStore.State.Shown)
    }

    @Test
    fun `without a source nothing is fetched`() = runBlocking {
        val store = store(null)
        store.load("turn:0", 0)
        assertEquals(0, fetches.get())
        assertNull(store.source)
    }

    @Test
    fun `the core's answer is read from base64 under agent image`() = runBlocking {
        var method = ""
        var args: JsonObject? = null
        val source = CorePromptImages { m, a ->
            method = m
            args = a
            buildJsonObject { put("base64", JsonPrimitive(java.util.Base64.getEncoder().encodeToString(byteArrayOf(1, 2, 3)))) }
        }
        assertEquals(listOf<Byte>(1, 2, 3), source.image("t1", "turn:4", 2).toList())
        assertEquals("agent.image", method)
        assertEquals("t1", args!!["terminal"]!!.jsonPrimitive.content)
        assertEquals("turn:4", args!!["row"]!!.jsonPrimitive.content)
        assertEquals("2", args!!["index"]!!.jsonPrimitive.content)
    }
}
