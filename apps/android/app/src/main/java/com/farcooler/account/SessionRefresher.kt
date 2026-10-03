package com.farcooler.account

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive

/**
 * What one call to the relay came back with.
 *
 * `post` used to answer `null` for all three, so "offline", "the relay is
 * having a bad minute" and "the relay refused your refresh token" were one
 * value, and the one thing done with it was to sign the person out.
 */
sealed interface RelayAnswer {
    /** A 200, with its JSON body. */
    data class Answered(val body: JsonObject) : RelayAnswer

    /** Any other status. The body when it parsed as a JSON object. */
    data class Refused(val status: Int, val body: JsonObject?) : RelayAnswer

    /** The request never completed: no network, DNS, a timeout, TLS. */
    data class Unreachable(val reason: String) : RelayAnswer
}

/** The body of a 200, or null for every other answer. */
val RelayAnswer.body: JsonObject?
    get() = (this as? RelayAnswer.Answered)?.body

/** How one refresh ended. */
sealed interface Refresh {
    /** A new access token, already stored. */
    data class Refreshed(val accessToken: String) : Refresh

    /** The relay said the refresh token is no longer good. The session is over. */
    data object SessionEnded : Refresh

    /** It didn't work this time, and the session is kept for the next try. */
    data class Failed(val reason: String) : Refresh
}

/**
 * One refresh at a time, and a signed-out phone only when the relay says so.
 *
 * A WorkOS refresh token is SINGLE USE: the response to `/v1/auth/refresh`
 * carries a new one and retires the token that was sent. A screen appearing
 * fires several relay calls at once, an expired access token sends every one
 * of them here, and each used to read the same stored token and spend it. The
 * first won; the rest were refused for a token that was valid when they read
 * it. AgentKit saw this live as three refreshes inside two seconds
 * (`Account.swift`, `refreshInFlight`). So the first caller makes the request
 * and every caller that arrives while it is out waits for that answer.
 *
 * Every refresh failure used to sign the person out. A phone waking without a
 * network, a relay deploy, a WorkOS outage: each one put the sign-in button
 * back and silently stopped push registration. Now only a definitive rejection
 * of the refresh token ends the session; see [rejectsRefreshToken].
 *
 * Closure-fed, so the rules are testable without a Context, the Keystore or a
 * relay.
 */
class SessionRefresher(
    /** The refresh token as stored now. */
    private val storedRefreshToken: () -> String?,
    /**
     * The stored access token if it is still good, else null. Asked again
     * once this caller holds the flight, because one that finished a moment
     * ago has already stored a new token, and a second refresh would only
     * spend another single-use one.
     */
    private val freshAccessToken: () -> String?,
    /** `POST /v1/auth/refresh` with this refresh token. */
    private val request: suspend (refreshToken: String) -> RelayAnswer,
    /** Keep what a successful refresh returned. */
    private val store: (JsonObject) -> Unit,
    /** Forget the session on this device. */
    private val endSession: () -> Unit,
) {
    private val lock = Mutex()
    private var inFlight: CompletableDeferred<Refresh>? = null

    /** Refresh, or join the refresh already out. */
    suspend fun refresh(): Refresh {
        val (flight, mine) = lock.withLock {
            val running = inFlight
            if (running != null) {
                running to false
            } else {
                CompletableDeferred<Refresh>().also { inFlight = it } to true
            }
        }
        if (!mine) return flight.await()

        val outcome = try {
            refreshing()
        } catch (cancelled: CancellationException) {
            // The caller that owned the flight went away. The ones waiting on
            // it have not, and must not wait forever.
            land(flight, Refresh.Failed("cancelled"))
            throw cancelled
        } catch (e: Exception) {
            Refresh.Failed(e.javaClass.simpleName)
        }
        land(flight, outcome)
        return outcome
    }

    /**
     * Empty the slot, then answer everyone waiting.
     *
     * Emptied first, so a caller arriving after the answer exists starts a new
     * refresh rather than joining one that has already finished.
     */
    private fun land(flight: CompletableDeferred<Refresh>, outcome: Refresh) {
        // `tryLock` cannot suspend, so this is safe in a cancelled coroutine.
        // The lock is only ever held for the check-and-set above, so it is free
        // here in practice; if it is not, clearing unguarded is still correct,
        // because only the owner of a flight ever clears it.
        val locked = lock.tryLock()
        try {
            if (inFlight === flight) inFlight = null
        } finally {
            if (locked) lock.unlock()
        }
        flight.complete(outcome)
    }

    private suspend fun refreshing(): Refresh {
        freshAccessToken()?.let { return Refresh.Refreshed(it) }
        val refreshToken = storedRefreshToken()
            ?: return Refresh.Failed("signed out")

        return when (val answer = request(refreshToken)) {
            is RelayAnswer.Answered -> {
                // Stored before anything else is checked: the token just sent
                // is spent, and the one in this body is the only one that works.
                store(answer.body)
                answer.body["accessToken"]?.jsonPrimitive?.contentOrNull
                    ?.let { Refresh.Refreshed(it) }
                    ?: Refresh.Failed("malformed response")
            }
            is RelayAnswer.Refused -> if (rejectsRefreshToken(answer)) {
                // Only if it is still the token that was refused: a sign-in
                // that finished while this was out has stored a new one.
                if (storedRefreshToken() == refreshToken) endSession()
                Refresh.SessionEnded
            } else {
                Refresh.Failed("status ${answer.status}")
            }
            is RelayAnswer.Unreachable -> Refresh.Failed(answer.reason)
        }
    }

    companion object {
        /**
         * Whether this answer says the refresh token itself is no good.
         *
         * Not "any 401". The relay answers EVERY failed WorkOS call with a 401
         * and puts WorkOS's own status in the body (`workosToken` in
         * `services/relay/src/index.ts`), so a WorkOS 503 reaches the phone as
         * a 401 too. Rejection is a 401 carrying a 4xx from WorkOS: the
         * refresh token was refused (`invalid_grant` is a 400). A 408 or 429
         * is WorkOS asking for time, not saying no, and a 401 with no status
         * in it says nothing about which happened.
         */
        fun rejectsRefreshToken(answer: RelayAnswer): Boolean {
            if (answer !is RelayAnswer.Refused || answer.status != 401) return false
            val upstream = (answer.body?.get("status") as? JsonPrimitive)?.intOrNull ?: return false
            return upstream in 400..499 && upstream != 408 && upstream != 429
        }
    }
}
