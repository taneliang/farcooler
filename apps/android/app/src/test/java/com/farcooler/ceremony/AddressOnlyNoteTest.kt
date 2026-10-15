package com.farcooler.ceremony

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

/** A ceremony reply that fails to decode is logged, not dropped silently, and never quoted (ov-530 R2-4, M1). */
class AddressOnlyNoteTest {
    @Test
    fun `a reply that does not decode names no one and is reported`() {
        val reported = mutableListOf<String>()
        val note = addressOnlyNoteFor(emptyList(), "{not json") { reported.add(it) }
        assertNull(note)
        assertEquals("a reply that failed to decode went unreported", 1, reported.size)
    }

    @Test
    fun `what is reported never carries the reply or the exception's message`() {
        val token = "tc-SECRETTOKEN-" + "t".repeat(40)
        val reply = """{"v":1,"runners":[{"id":"x","reach":{"token":"$token""" // cut off mid-token
        val reported = mutableListOf<String>()
        addressOnlyNoteFor(emptyList(), reply) { reported.add(it) }
        assertEquals(1, reported.size)
        val text = reported.single()
        assertFalse("the token reached the log: $text", text.contains("SECRETTOKEN"))
        assertFalse("the input reached the log: $text", text.contains("runners"))
        assertFalse("a message reached the log: $text", text.contains(" "))
    }
}
