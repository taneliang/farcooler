package com.farcooler.account

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.yield
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A network blip doesn't sign you out, and two callers spend one refresh token.
 *
 * Android's `Account.accessToken` signed the person out on any failed refresh
 * and let every concurrent caller spend the same single-use token. AgentKit
 * fixed both (`Account.swift`, `refreshInFlight` and `refreshingAccessToken`).
 */
class SessionRefresherTest {

    /** A stand-in for the Keystore and the relay, recording what was done. */
    private class Fixture(var answer: suspend (String) -> RelayAnswer) {
        var refreshToken: String? = "rt-1"
        var access: String? = null
        var requests = mutableListOf<String>()
        var ended = 0

        val refresher = SessionRefresher(
            storedRefreshToken = { refreshToken },
            freshAccessToken = { access },
            request = { token -> requests += token; answer(token) },
            store = { body ->
                (body["accessToken"] as? JsonPrimitive)?.content?.let { access = it }
                (body["refreshToken"] as? JsonPrimitive)?.content?.let { refreshToken = it }
            },
            endSession = { ended++; refreshToken = null; access = null },
        )
    }

    private fun minted(access: String, refresh: String) = RelayAnswer.Answered(
        buildJsonObject {
            put("accessToken", JsonPrimitive(access))
            put("refreshToken", JsonPrimitive(refresh))
        }
    )

    /** The relay's 401 for a failed WorkOS call, carrying WorkOS's status. */
    private fun workosRefused(status: Int) = RelayAnswer.Refused(
        401,
        buildJsonObject {
            put("error", JsonPrimitive("auth"))
            put("status", JsonPrimitive(status))
        },
    )

    /**
     * Two callers at once make one request, and both get its token.
     *
     * Mutation: no single flight (each caller requests). Red: two requests,
     * the second spending `rt-1` after the first retired it.
     */
    @Test
    fun twoConcurrentCallersSpendTheRefreshTokenOnce() = runTest {
        val gate = CompletableDeferred<Unit>()
        val fixture = Fixture { gate.await(); minted("at-2", "rt-2") }

        val first = async { fixture.refresher.refresh() }
        val second = async { fixture.refresher.refresh() }
        yield()
        gate.complete(Unit)
        val outcomes = listOf(first, second).awaitAll()

        assertEquals(listOf("rt-1"), fixture.requests)
        assertEquals(listOf(Refresh.Refreshed("at-2"), Refresh.Refreshed("at-2")), outcomes)
        assertEquals("rt-2", fixture.refreshToken)
    }

    /**
     * A caller arriving after a flight has landed uses the token it stored,
     * rather than spending the new refresh token for nothing.
     */
    @Test
    fun aCallerAfterTheFlightUsesTheTokenItStored() = runTest {
        val fixture = Fixture { minted("at-2", "rt-2") }
        assertEquals(Refresh.Refreshed("at-2"), fixture.refresher.refresh())
        assertEquals(Refresh.Refreshed("at-2"), fixture.refresher.refresh())
        assertEquals(listOf("rt-1"), fixture.requests)
    }

    /**
     * Offline: the session stays.
     *
     * Mutation: any failure ends the session (the old `body == null` arm).
     * Red: `ended` is 1 and the refresh token is gone.
     */
    @Test
    fun anUnreachableRelayKeepsTheSession() = runTest {
        val fixture = Fixture { RelayAnswer.Unreachable("offline") }
        assertEquals(Refresh.Failed("offline"), fixture.refresher.refresh())
        assertEquals(0, fixture.ended)
        assertEquals("rt-1", fixture.refreshToken)
    }

    /** The relay itself failing, or WorkOS failing behind it, keeps the session. */
    @Test
    fun aServerFailureKeepsTheSession() = runTest {
        for (answer in listOf(
            RelayAnswer.Refused(500, null),
            RelayAnswer.Refused(503, null),
            RelayAnswer.Refused(429, null),
            // WorkOS down: the relay still says 401, with WorkOS's 5xx inside.
            workosRefused(503),
            workosRefused(429),
            // A 401 that names no upstream status says nothing about the token.
            RelayAnswer.Refused(401, null),
        )) {
            val fixture = Fixture { answer }
            val outcome = fixture.refresher.refresh()
            assertTrue("$answer must fail, got $outcome", outcome is Refresh.Failed)
            assertEquals("$answer must keep the session", 0, fixture.ended)
            assertEquals("rt-1", fixture.refreshToken)
        }
    }

    /**
     * WorkOS refused the refresh token: the session is over, and the sign-in
     * button comes back rather than every later call failing silently.
     *
     * Mutation: never end the session. Red: `ended` stays 0.
     */
    @Test
    fun aRejectedRefreshTokenEndsTheSession() = runTest {
        for (status in listOf(400, 401)) {
            val fixture = Fixture { workosRefused(status) }
            assertEquals(Refresh.SessionEnded, fixture.refresher.refresh())
            assertEquals(1, fixture.ended)
            assertNull(fixture.refreshToken)
        }
    }

    /** A sign-in that landed while the refused refresh was out is not undone. */
    @Test
    fun aRejectionDoesNotUndoANewerSignIn() = runTest {
        lateinit var fixture: Fixture
        fixture = Fixture { fixture.refreshToken = "rt-new"; workosRefused(400) }
        assertEquals(Refresh.SessionEnded, fixture.refresher.refresh())
        assertEquals(0, fixture.ended)
        assertEquals("rt-new", fixture.refreshToken)
    }

    /**
     * A failed flight is not remembered: the next caller tries again.
     *
     * Mutation: the slot never cleared. Red: the second call joins the
     * finished flight and makes no request.
     */
    @Test
    fun aFailedFlightIsNotReusedByTheNextCaller() = runTest {
        var answers = listOf<RelayAnswer>(RelayAnswer.Unreachable("offline"), minted("at-2", "rt-2"))
        val fixture = Fixture { answers.first().also { answers = answers.drop(1) } }
        assertEquals(Refresh.Failed("offline"), fixture.refresher.refresh())
        assertEquals(Refresh.Refreshed("at-2"), fixture.refresher.refresh())
        assertEquals(2, fixture.requests.size)
    }

    /** A 200 with no access token keeps the new refresh token it did carry. */
    @Test
    fun aMalformedAnswerStillStoresTheNewRefreshToken() = runTest {
        val fixture = Fixture {
            RelayAnswer.Answered(buildJsonObject { put("refreshToken", JsonPrimitive("rt-2")) })
        }
        assertEquals(Refresh.Failed("malformed response"), fixture.refresher.refresh())
        assertEquals("rt-2", fixture.refreshToken)
        assertEquals(0, fixture.ended)
    }

    @Test
    fun onlyAWorkOSRejectionInsideA401IsDefinitive() {
        assertTrue(SessionRefresher.rejectsRefreshToken(workosRefused(400)))
        assertFalse(SessionRefresher.rejectsRefreshToken(workosRefused(500)))
        assertFalse(SessionRefresher.rejectsRefreshToken(RelayAnswer.Refused(400, JsonObject(emptyMap()))))
        assertFalse(
            SessionRefresher.rejectsRefreshToken(
                RelayAnswer.Refused(401, buildJsonObject { put("status", JsonObject(emptyMap())) })
            )
        )
        assertFalse(SessionRefresher.rejectsRefreshToken(RelayAnswer.Unreachable("offline")))
    }

    /**
     * An Error, not an Exception, from the request: the flight still lands,
     * so the next caller is not left waiting on it forever.
     *
     * Mutation: `land` outside a `finally`, after `catch (e: Exception)`.
     * Red: the second call never returns, and the test times out.
     */
    @Test
    fun anErrorStillLandsTheFlight() = runTest {
        var first = true
        val fixture = Fixture {
            if (first) { first = false; throw AssertionError("keystore") }
            minted("at-2", "rt-2")
        }
        val thrown = runCatching { fixture.refresher.refresh() }.exceptionOrNull()
        assertTrue("$thrown", thrown is AssertionError)
        assertEquals(Refresh.Refreshed("at-2"), withTimeout(1_000) { fixture.refresher.refresh() })
    }

    /**
     * The owner of a flight is cancelled: whoever was waiting on it gets a
     * failure rather than waiting forever, and the next call refreshes afresh.
     */
    @Test
    fun aCancelledOwnerReleasesItsWaiters() = runTest {
        val gate = CompletableDeferred<Unit>()
        var calls = 0
        val fixture = Fixture {
            calls++
            if (calls == 1) gate.await()
            minted("at-2", "rt-2")
        }
        val owner = async { fixture.refresher.refresh() }
        yield()
        val waiter = async { fixture.refresher.refresh() }
        yield()
        owner.cancel()
        assertEquals(Refresh.Failed("cancelled"), waiter.await())
        assertEquals(Refresh.Refreshed("at-2"), fixture.refresher.refresh())
        assertEquals(2, calls)
        assertEquals(0, fixture.ended)
    }
}
