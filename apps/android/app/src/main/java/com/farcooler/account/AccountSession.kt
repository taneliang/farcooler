package com.farcooler.account

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.net.HttpURLConnection
import java.net.URL
import java.util.Base64

/**
 * The part of [Account] that decides whether there is an access token, with
 * its storage and its relay passed in.
 *
 * [Account] needs a Context and the Keystore, so a JVM test cannot build one.
 * This is the code [Account.accessToken] runs, unchanged, so a test of it is a
 * test of the call site and not only of [SessionRefresher]'s rules.
 */
class AccountSession(
    /** A stored credential by key: [Account.KEY_ACCESS] or [Account.KEY_REFRESH]. */
    private val readToken: (key: String) -> String?,
    /** One POST to the relay. */
    private val post: suspend (path: String, body: JsonObject) -> RelayAnswer,
    /** Keep what a successful refresh returned. */
    private val store: (JsonObject) -> Unit,
    /** Forget the session on this device. */
    private val forget: () -> Unit,
    private val now: () -> Long = System::currentTimeMillis,
) {
    private val refresher = SessionRefresher(
        storedRefreshToken = { readToken(Account.KEY_REFRESH) },
        freshAccessToken = ::freshAccessToken,
        request = { refresh ->
            post("/v1/auth/refresh", buildJsonObject { put("refreshToken", JsonPrimitive(refresh)) })
        },
        store = store,
        // A refresh token the relay refused means the session is over, and
        // leaving a dead one in place makes every later call fail silently
        // instead of showing a sign-in button.
        endSession = forget,
    )

    /**
     * The stored access token while it has a minute left, else a refreshed
     * one, else null for this call.
     *
     * Null with the session kept when the refresh failed: an unreachable relay
     * is not a signed-out person. Only a refresh token the relay definitively
     * refused ends it; see [SessionRefresher].
     */
    suspend fun accessToken(): String? {
        if (readToken(Account.KEY_REFRESH) == null) return null
        freshAccessToken()?.let { return it }
        return (refresher.refresh() as? Refresh.Refreshed)?.accessToken
    }

    /** The stored access token, if it has more than a minute left. */
    private fun freshAccessToken(): String? {
        val access = readToken(Account.KEY_ACCESS) ?: return null
        val expiry = jwtExpiry(access) ?: return null
        return access.takeIf { expiry - now() > 60_000 }
    }

    companion object {
        private val json = Json { ignoreUnknownKeys = true }

        /**
         * Read a JWT's `exp` without verifying it.
         *
         * Verification is the relay's job — it has the JWKS. This only decides
         * whether to bother sending a token that is already stale, and a forged
         * expiry buys nothing but an extra refresh.
         */
        fun jwtExpiry(token: String): Long? {
            val parts = token.split(".")
            if (parts.size != 3) return null
            return runCatching {
                val payload = Base64.getUrlDecoder().decode(parts[1])
                val claims = json.parseToJsonElement(String(payload)).jsonObject
                (claims["exp"]?.jsonPrimitive?.doubleOrNull ?: return null).toLong() * 1000
            }.getOrNull()
        }
    }
}

/**
 * One blocking POST to the relay at [url], and which of three things happened.
 *
 * It used to answer null for a refusal, a 5xx and no network alike, and the
 * refresh path read that one null as "the session is over". Blocking, and a
 * plain function, so a test can point it at a local server.
 */
fun relayPost(url: String, body: JsonObject, bearer: String? = null): RelayAnswer {
    val connection = try {
        // `relay` is a setting anyone can type into, so a stray space in it
        // must surface as a relay that would not answer rather than a crash.
        (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            setRequestProperty("Content-Type", "application/json")
            bearer?.let { setRequestProperty("Authorization", "Bearer $it") }
            connectTimeout = 15_000
            readTimeout = 15_000
            doOutput = true
        }
    } catch (e: Exception) {
        return RelayAnswer.Unreachable("relay address invalid")
    }
    val status = try {
        connection.outputStream.use { it.write(body.toString().toByteArray()) }
        connection.responseCode
    } catch (e: Exception) {
        // The request never completed. Named by its class, never its message,
        // which can carry the URL.
        return RelayAnswer.Unreachable(e.javaClass.simpleName)
    }
    val text = runCatching {
        (if (status == 200) connection.inputStream else connection.errorStream)
            ?.bufferedReader()?.readText()
    }.getOrNull()
    val parsed = text?.let { runCatching { Json.parseToJsonElement(it).jsonObject }.getOrNull() }
    return when {
        status != 200 -> RelayAnswer.Refused(status, parsed)
        // A 200 that isn't JSON is the relay's failure, not this session's.
        parsed == null -> RelayAnswer.Refused(status, null)
        else -> RelayAnswer.Answered(parsed)
    }
}
