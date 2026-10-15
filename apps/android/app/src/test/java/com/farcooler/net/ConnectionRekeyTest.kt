package com.farcooler.net

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Which key a connection offers a runner during "Make a new key" (ov-569),
 * and what a refusal of it teaches the phone.
 */
class ConnectionRekeyTest {
    private class Ring(override val previous: String?, override var onPrevious: Set<String>) : KeyRing {
        override val current = "new"
    }

    private class Refused : Exception("key_rejected")

    private suspend fun connectAs(ring: Ring, accepts: Set<String>, runner: String = "r1"): List<String> {
        val offered = mutableListOf<String>()
        connectWithEitherKey(runner, ring, { key ->
            offered += key
            if (key !in accepts) throw Refused()
        }, refused = { it is Refused })
        return offered
    }

    @Test
    fun aRunnerNotMovedIsReachedWithTheOldKey() = runTest {
        val ring = Ring(previous = "old", onPrevious = setOf("r1"))
        assertEquals(listOf("old"), connectAs(ring, accepts = setOf("old")))
        assertEquals(setOf("r1"), ring.onPrevious)
    }

    @Test
    fun aRunnerThatMovedWhenItsAnswerWasLostIsReachedWithTheNewKeyAndMarkedMoved() = runTest {
        val ring = Ring(previous = "old", onPrevious = setOf("r1"))
        assertEquals(listOf("old", "new"), connectAs(ring, accepts = setOf("new")))
        assertTrue(ring.onPrevious.isEmpty())
    }

    @Test
    fun aRunnerThatNeverMovedIsReachedWithTheOldKeyAndMarkedToMove() = runTest {
        val ring = Ring(previous = "old", onPrevious = emptySet())
        assertEquals(listOf("new", "old"), connectAs(ring, accepts = setOf("old")))
        assertEquals(setOf("r1"), ring.onPrevious)
    }

    @Test
    fun withNoOldKeyARefusalIsJustARefusal() = runTest {
        val ring = Ring(previous = null, onPrevious = setOf("r1"))
        val failure = runCatching { connectAs(ring, accepts = emptySet()) }.exceptionOrNull()
        assertTrue(failure is Refused)
    }

    @Test
    fun aFailureThatIsntARefusalTriesNothingElse() = runTest {
        val ring = Ring(previous = "old", onPrevious = emptySet())
        val offered = mutableListOf<String>()
        val failure = runCatching {
            connectWithEitherKey("r1", ring, { offered += it; throw IllegalStateException("unreachable") }, refused = { it is Refused })
        }.exceptionOrNull()
        assertTrue(failure is IllegalStateException)
        assertEquals(listOf("new"), offered)
    }
}
