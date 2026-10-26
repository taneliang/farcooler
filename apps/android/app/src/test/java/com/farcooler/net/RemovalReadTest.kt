package com.farcooler.net

import com.farcooler.core.CoreException
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** A removed member's phone is told so (ov-784): a refusal as someone else's asks `member.list`, which says. */
class RemovalReadTest {
    private fun listed(you: String) = JsonObject(mapOf("members" to JsonArray(emptyList()), "you" to JsonPrimitive(you)))
    private val notYours = CoreException("not yours", word = "not-yours", what = "not_yours")

    @Test
    fun aRefusalAsSomeoneElsesAsksAndTheListDecides() = runTest {
        var you = "(released)"
        var calls = 0
        var clock = 0L
        val read = RemovalRead({ method, _ ->
            calls += 1
            assertEquals("member.list", method)
            listed(you)
        }, now = { clock })
        assertTrue(read.notice(notYours))
        assertTrue(read.removed.value)
        // Within the wait, asked nothing more.
        clock += 5_000
        assertTrue(read.notice(notYours))
        assertEquals(1, calls)
        // Check again asks at once; added back, it isn't removed.
        you = "bob"
        read.checkAgain()
        assertFalse(read.notice(notYours))
        assertFalse(read.removed.value)
        assertEquals(2, calls)
    }

    @Test
    fun anyOtherFailureAsksNothing() = runTest {
        var calls = 0
        val read = RemovalRead({ _, _ ->
            calls += 1
            listed("(released)")
        })
        assertFalse(read.notice(CoreException("gone", word = "not-found")))
        assertFalse(read.notice(RuntimeException("link")))
        assertEquals(0, calls)
        assertFalse(read.removed.value)
    }

    @Test
    fun aFleetThatLandsEndsIt() = runTest {
        val read = RemovalRead({ _, _ -> listed("(released)") })
        assertTrue(read.notice(notYours))
        read.landed()
        assertFalse(read.removed.value)
    }

    @Test
    fun aListThatFailsLeavesItAsItWas() = runTest {
        val read = RemovalRead({ _, _ -> throw CoreException("no") })
        assertFalse(read.notice(notYours))
        assertFalse(read.removed.value)
    }
}
