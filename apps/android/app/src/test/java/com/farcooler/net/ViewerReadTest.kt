package com.farcooler.net

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Who this phone is, from `member.list`'s `you` (ov-597). */
class ViewerReadTest {
    @Test
    fun readsYouOnceAndKeepsIt() = runBlocking {
        var calls = 0
        val read = ViewerRead { method, _ ->
            calls += 1
            assertEquals("member.list", method)
            JsonObject(mapOf("you" to JsonPrimitive("alex")))
        }
        assertNull(read.you.value)
        read.read()
        read.read()
        assertEquals("alex", read.you.value)
        assertEquals(1, calls)
    }

    @Test
    fun theOwnersEmptyYouIsKnownNotMissing() = runBlocking {
        val read = ViewerRead { _, _ -> JsonObject(mapOf("you" to JsonPrimitive(""))) }
        read.read()
        assertEquals("", read.you.value)
    }

    @Test
    fun aReplyWithNoYouLeavesItUnknown() = runBlocking {
        val read = ViewerRead { _, _ -> JsonObject(emptyMap()) }
        read.read()
        assertNull(read.you.value)
    }
}
