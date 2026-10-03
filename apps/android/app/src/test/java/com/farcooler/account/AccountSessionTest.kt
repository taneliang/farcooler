package com.farcooler.account

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.yield
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.util.Base64

/**
 * [Account.accessToken]'s code, with the Keystore and the relay faked.
 * [SessionRefresherTest] proves the rules; this proves the call site uses them.
 */
class AccountSessionTest {

    private class Device(var answer: suspend () -> RelayAnswer) {
        val tokens = mutableMapOf<String, String>()
        val posts = mutableListOf<Pair<String, JsonObject>>()
        var forgotten = 0

        val session = AccountSession(
            readToken = { tokens[it] },
            post = { path, body -> posts += path to body; answer() },
            store = { body ->
                (body["accessToken"] as? JsonPrimitive)?.content?.let { tokens[Account.KEY_ACCESS] = it }
                (body["refreshToken"] as? JsonPrimitive)?.content?.let { tokens[Account.KEY_REFRESH] = it }
            },
            forget = { forgotten++; tokens.clear() },
            now = { NOW },
        )

        init {
            tokens[Account.KEY_REFRESH] = "rt-1"
            tokens[Account.KEY_ACCESS] = jwt(expiresAt = NOW / 1000 - 10)
        }
    }

    companion object {
        const val NOW = 1_790_000_000_000L

        fun jwt(expiresAt: Long): String {
            val payload = Base64.getUrlEncoder().withoutPadding()
                .encodeToString("""{"exp":$expiresAt}""".toByteArray())
            return "e30.$payload.sig"
        }

        val rejected = RelayAnswer.Refused(
            401,
            buildJsonObject { put("error", JsonPrimitive("auth")); put("status", JsonPrimitive(400)) },
        )
    }

    /**
     * Offline: no token for this call, and still signed in.
     *
     * Mutation: `accessToken` back to forgetting on any failed refresh. Red.
     */
    @Test
    fun offlineKeepsTheSession() = runTest {
        val device = Device { RelayAnswer.Unreachable("offline") }
        assertNull(device.session.accessToken())
        assertEquals(0, device.forgotten)
        assertEquals("rt-1", device.tokens[Account.KEY_REFRESH])
        assertEquals("/v1/auth/refresh", device.posts.single().first)
        assertEquals(JsonPrimitive("rt-1"), device.posts.single().second["refreshToken"])
    }

    @Test
    fun aRelayFailureKeepsTheSession() = runTest {
        val device = Device { RelayAnswer.Refused(503, null) }
        assertNull(device.session.accessToken())
        assertEquals(0, device.forgotten)
    }

    @Test
    fun aRejectedRefreshTokenSignsOut() = runTest {
        val device = Device { rejected }
        assertNull(device.session.accessToken())
        assertEquals(1, device.forgotten)
    }

    /**
     * Mutation: `accessToken` calling the relay directly, without the
     * refresher. Red: two posts.
     */
    @Test
    fun twoCallersRefreshOnce() = runTest {
        val gate = CompletableDeferred<Unit>()
        val device = Device {
            gate.await()
            RelayAnswer.Answered(
                buildJsonObject {
                    put("accessToken", JsonPrimitive("at-2"))
                    put("refreshToken", JsonPrimitive("rt-2"))
                }
            )
        }
        val calls = listOf(async { device.session.accessToken() }, async { device.session.accessToken() })
        yield()
        gate.complete(Unit)
        assertEquals(listOf("at-2", "at-2"), calls.awaitAll())
        assertEquals(1, device.posts.size)
    }

    /** A token with more than a minute left is used without asking anyone. */
    @Test
    fun aFreshTokenIsUsedAsIs() = runTest {
        val device = Device { error("must not refresh") }
        val fresh = jwt(expiresAt = NOW / 1000 + 3600)
        device.tokens[Account.KEY_ACCESS] = fresh
        assertEquals(fresh, device.session.accessToken())
        assertEquals(0, device.posts.size)
    }

    @Test
    fun noRefreshTokenIsNotSignedIn() = runTest {
        val device = Device { error("must not refresh") }
        device.tokens.clear()
        assertNull(device.session.accessToken())
        assertEquals(0, device.posts.size)
    }
}
