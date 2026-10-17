package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.model.RunnerLink
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Who this phone is, from `member.list`'s `you`, read once per connection (ov-597, ov-598). */
@OptIn(ExperimentalCoroutinesApi::class)
class ViewerReadTest {
    private fun you(name: String) = JsonObject(mapOf("you" to JsonPrimitive(name)))

    private fun TestScope.reader(
        link: MutableStateFlow<RunnerLink> = MutableStateFlow(RunnerLink.ANSWERING),
        scope: CoroutineScope = this,
        waits: (Int) -> Long = { 1_000L },
        call: suspend (String, JsonObject) -> JsonObject,
    ) = ViewerRead(call, link, scope, waits)

    @Test
    fun readsYouOnceAndKeepsIt() = runTest {
        var calls = 0
        val read = reader { method, _ ->
            calls += 1
            assertEquals("member.list", method)
            you("alex")
        }
        assertNull(read.you.value)
        read.ask()
        runCurrent()
        read.ask()
        runCurrent()
        assertEquals("alex", read.you.value)
        assertEquals(1, calls)
    }

    @Test
    fun theOwnersEmptyYouIsKnownNotMissing() = runTest {
        val read = reader { _, _ -> you("") }
        read.ask()
        runCurrent()
        assertEquals("", read.you.value)
    }

    @Test
    fun aReplyWithNoYouLeavesItUnknownAndIsNotRepeated() = runTest {
        var calls = 0
        val read = reader { _, _ -> calls += 1; JsonObject(emptyMap()) }
        read.ask()
        runCurrent()
        read.ask()
        advanceTimeBy(120_000)
        runCurrent()
        assertNull(read.you.value)
        assertEquals(1, calls)
    }

    @Test
    fun panesAskingTogetherMakeOneCall() = runTest {
        var calls = 0
        val read = reader { _, _ -> calls += 1; kotlinx.coroutines.yield(); you("alex") }
        repeat(5) { read.ask() }
        runCurrent()
        assertEquals("alex", read.you.value)
        assertEquals(1, calls)
    }

    @Test
    fun aFailedReadIsTriedAgainAfterAGrowingWait() = runTest {
        val at = mutableListOf<Long>()
        val read = reader(waits = { attempt -> 1_000L shl attempt }) { _, _ ->
            at += testScheduler.currentTime
            if (at.size < 3) throw CoreException("dropped")
            you("alex")
        }
        read.ask()
        runCurrent()
        assertNull(read.you.value)
        advanceTimeBy(1_000)
        runCurrent()
        advanceTimeBy(2_000)
        runCurrent()
        assertEquals(listOf(0L, 1_000L, 3_000L), at)
        assertEquals("alex", read.you.value)
    }

    @Test
    fun noCallIsMadeWhileTheConnectionIsAwayAndOneIsMadeWhenItReturns() = runTest {
        val link = MutableStateFlow(RunnerLink.AWAY)
        var calls = 0
        val read = reader(link) { _, _ -> calls += 1; you("alex") }
        read.ask()
        runCurrent()
        advanceTimeBy(120_000)
        runCurrent()
        assertEquals(0, calls)
        link.value = RunnerLink.ANSWERING
        runCurrent()
        assertEquals(1, calls)
        assertEquals("alex", read.you.value)
    }

    @Test
    fun aRetryAfterAnOutageGoesOutAsSoonAsTheLinkIsBackNotAfterTheWait() = runTest {
        val link = MutableStateFlow(RunnerLink.ANSWERING)
        var calls = 0
        val read = reader(link, waits = { 30_000L }) { _, _ ->
            calls += 1
            if (calls == 1) {
                link.value = RunnerLink.AWAY
                throw CoreException("dropped")
            }
            you("alex")
        }
        read.ask()
        runCurrent()
        advanceTimeBy(10_000)
        runCurrent()
        assertEquals(1, calls)
        val back = testScheduler.currentTime
        link.value = RunnerLink.ANSWERING
        runCurrent()
        assertEquals(back, testScheduler.currentTime)
        assertEquals(2, calls)
        assertEquals("alex", read.you.value)
    }

    @Test
    fun theReadEndsWithTheConnectionsScopeNotWithThePaneThatAsked() = runTest {
        val connection = Job()
        val link = MutableStateFlow(RunnerLink.AWAY)
        var calls = 0
        val read = reader(link, scope = CoroutineScope(connection + coroutineContext.minusKey(Job))) { _, _ ->
            calls += 1
            you("alex")
        }
        val job = read.ask()
        runCurrent()
        assertTrue(job?.isActive == true)
        connection.cancel()
        runCurrent()
        link.value = RunnerLink.ANSWERING
        runCurrent()
        assertEquals(0, calls)
        assertTrue(job?.isCancelled == true)
    }
}
