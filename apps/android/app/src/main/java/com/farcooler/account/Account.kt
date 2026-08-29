package com.farcooler.account

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Base64
import androidx.browser.customtabs.CustomTabsIntent
import com.farcooler.BuildConfig
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.security.SecureRandom

/**
 * Who you are, so a runner you own can reach a phone you carry.
 *
 * This is the ONLY place in the Android app that knows about WorkOS, matching
 * the Apple apps. A daemon signs in to nothing: it holds an opaque token this
 * app asked the relay for and handed over SSH, so a headless Linux box never
 * needs a browser and a stolen daemon token is worth exactly one thing —
 * notifying the account it was stolen from.
 *
 * The app carries a WorkOS client id, which is public by design. It never
 * carries the API key. The code exchange happens on the relay because that is
 * the single step that needs a secret, and a secret in an open-source repo or
 * an unzippable APK is not a secret.
 *
 * ## Custom Tabs, not a WebView
 *
 * The same reasoning as `ASWebAuthenticationSession` on Apple: a Custom Tab is
 * the browser, so it reaches the sign-in state the user already has, and this
 * app cannot read what is typed into it. A WebView would mean Far Cooler could
 * read the password, which is the thing outsourcing auth was meant to avoid.
 */
class Account(context: Context) {
    private val appContext = context.applicationContext
    private val preferences =
        appContext.getSharedPreferences("farcooler.account", Context.MODE_PRIVATE)
    private val tokens = TokenStore(appContext)

    private val _email = MutableStateFlow(preferences.getString(KEY_EMAIL, "").orEmpty())
    val email: StateFlow<String> = _email.asStateFlow()

    private val _userId = MutableStateFlow(preferences.getString(KEY_USER_ID, "").orEmpty())
    val userId: StateFlow<String> = _userId.asStateFlow()

    private val _signingIn = MutableStateFlow(false)
    val signingIn: StateFlow<Boolean> = _signingIn.asStateFlow()

    private val _lastError = MutableStateFlow<String?>(null)
    val lastError: StateFlow<String?> = _lastError.asStateFlow()

    val isSignedIn: Boolean get() = _userId.value.isNotEmpty()

    /** Called once a sign-in completes, so the push token can be filed. */
    var afterSignIn: (() -> Unit)? = null

    /**
     * Where the relay lives. A setting so self-hosting is configuration rather
     * than a fork, and so a development build can point at `wrangler dev`.
     */
    var relay: String
        get() = preferences.getString(KEY_RELAY, null) ?: DEFAULT_RELAY
        set(value) {
            preferences.edit().putString(KEY_RELAY, value).apply()
        }

    /**
     * The AuthKit client id for this project. Public: it names the app, not the
     * bearer, and every OAuth public client ships one.
     */
    val clientId: String
        get() = preferences.getString(KEY_CLIENT_ID, null)
            ?: BuildConfig.WORKOS_CLIENT_ID

    private val json = Json { ignoreUnknownKeys = true }

    /**
     * The verifier for the sign-in currently in flight, and the state bound to
     * it.
     *
     * Held rather than passed through the redirect: the callback arrives as a
     * fresh intent on the activity, which is not a continuation of anything.
     * PKCE already defeats the practical code-injection attack — an injected
     * code was issued against someone else's challenge — but `farcooler://` is
     * a scheme any app on the device may claim, and a flow with no request
     * binding of its own has nothing to say about a callback it never asked
     * for.
     */
    private var pendingVerifier: String? = null
    private var pendingState: String? = null

    // MARK: - Signing in

    /** Open AuthKit in the browser. The answer arrives through [handleCallback]. */
    fun signIn(context: Context) {
        if (clientId.isEmpty()) {
            _lastError.value =
                "This build has no WorkOS client id. Set farcooler.workosClientId when building."
            return
        }
        val verifier = randomVerifier()
        val challenge = verifier?.let(::challengeFor)
        val state = randomVerifier()
        if (verifier == null || challenge == null || state == null) {
            _lastError.value = "Could not start sign-in."
            return
        }
        pendingVerifier = verifier
        pendingState = state
        _signingIn.value = true
        _lastError.value = null

        val url = Uri.parse("https://api.workos.com/user_management/authorize")
            .buildUpon()
            .appendQueryParameter("client_id", clientId)
            .appendQueryParameter("redirect_uri", REDIRECT_URI)
            .appendQueryParameter("response_type", "code")
            .appendQueryParameter("provider", "authkit")
            .appendQueryParameter("code_challenge", challenge)
            .appendQueryParameter("code_challenge_method", "S256")
            .appendQueryParameter("state", state)
            .build()

        runCatching {
            CustomTabsIntent.Builder()
                .setShowTitle(false)
                .build()
                .also { it.intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK) }
                .launchUrl(context, url)
        }.onFailure {
            _signingIn.value = false
            _lastError.value = "No browser on this device could open the sign-in page."
        }
    }

    /**
     * Finish a sign-in from the redirect.
     *
     * Returns whether the URI was ours, so the caller can tell a callback from
     * an unrelated deep link.
     */
    suspend fun handleCallback(uri: Uri): Boolean {
        if (uri.scheme != SCHEME) return false
        _signingIn.value = false

        val code = uri.getQueryParameter("code")
        val state = uri.getQueryParameter("state")
        val verifier = pendingVerifier
        pendingVerifier = null
        val expected = pendingState
        pendingState = null

        if (code == null || verifier == null) {
            // Someone closing the tab is someone changing their mind, and
            // telling them they failed at it is obnoxious. Only say something
            // when a code came back and could not be used.
            if (code != null) _lastError.value = "Sign-in did not complete."
            return true
        }
        if (state != expected) {
            _lastError.value = "Sign-in did not complete."
            return true
        }

        val body = post("/v1/auth/token", buildJsonObject {
            put("code", JsonPrimitive(code))
            put("verifier", JsonPrimitive(verifier))
        })
        if (body == null) {
            _lastError.value = "The relay would not complete the sign-in."
            return true
        }
        store(body)
        afterSignIn?.invoke()
        return true
    }

    /**
     * Forget the session on this device.
     *
     * Paired runners keep working: they hold tokens, not your session, and
     * signing out of a phone should not silence the fleet.
     */
    suspend fun signOut() {
        // Told to the relay first, while there is still a token to tell it
        // with. Clearing locally alone left the refresh token valid until
        // natural expiry — so anyone who had lifted it kept minting sessions
        // after the user believed they had signed out.
        tokens.read(KEY_REFRESH)?.let { refresh ->
            post("/v1/auth/logout", buildJsonObject { put("refreshToken", JsonPrimitive(refresh)) })
        }
        forgetLocally()
    }

    private fun forgetLocally() {
        _userId.value = ""
        _email.value = ""
        preferences.edit().remove(KEY_USER_ID).remove(KEY_EMAIL).apply()
        tokens.delete(KEY_ACCESS)
        tokens.delete(KEY_REFRESH)
    }

    // MARK: - Talking to the relay

    /**
     * A valid access token, refreshing first if the stored one has expired.
     *
     * Refresh happens through the relay for the same reason the exchange does:
     * WorkOS wants the API key on that call, and an app that could refresh
     * alone would be an app carrying the key.
     */
    suspend fun accessToken(): String? {
        val refresh = tokens.read(KEY_REFRESH) ?: return null
        val access = tokens.read(KEY_ACCESS)
        if (access != null) {
            val expiry = jwtExpiry(access)
            if (expiry != null && expiry - System.currentTimeMillis() > 60_000) return access
        }
        val body = post("/v1/auth/refresh", buildJsonObject {
            put("refreshToken", JsonPrimitive(refresh))
        })
        if (body == null) {
            // A refresh token that no longer works means the session is over,
            // and leaving a dead one in place makes every later call fail
            // silently instead of showing a sign-in button.
            forgetLocally()
            return null
        }
        store(body)
        return body["accessToken"]?.jsonPrimitive?.contentOrNull
    }

    /**
     * Tell the relay where to reach this device, and what version is asking.
     *
     * Returns whether it worked. The failure mode is the product's central
     * promise going quietly missing: the device is never filed, the daemon's
     * pushes reach zero addresses, and the settings screen goes on saying
     * notifications can reach this device.
     */
    suspend fun registerDevice(
        pushToken: String,
        platform: String,
        label: String,
        notifyOnDone: Boolean = true,
    ): Boolean {
        val token = accessToken() ?: return false
        val body = post(
            "/v1/devices",
            buildJsonObject {
                put("pushToken", JsonPrimitive(pushToken))
                put("platform", JsonPrimitive(platform))
                put("label", JsonPrimitive(label))
                // So the devices screen can show which of your runners is
                // behind, without anyone having to go and look.
                put("version", JsonPrimitive(AppVersion.reported))
                // "When an agent finishes or fails", so the toggle reaches the
                // pushes too. It used to be read only by this app's own
                // `Notifier`, which runs when the app is running — the case the
                // product is not about. With the phone asleep the tray card
                // comes from the relay, and the relay had never heard of the
                // setting: silence with the app open, banners with the phone in
                // a pocket.
                //
                // Always sent, never omitted. The relay COALESCEs an absent
                // field into what it already holds, which is right for a build
                // too old to know about this and wrong for one turning the
                // setting back ON.
                put("notifyOnDone", JsonPrimitive(notifyOnDone))
            },
            bearer = token,
        )
        return body != null
    }

    /**
     * Ask for a token that lets one runner notify this account.
     *
     * Returned once and stored on the relay only as a hash, so this is the only
     * moment it exists in readable form — hand it straight to the runner.
     */
    suspend fun pairDaemon(label: String): String? {
        val token = accessToken() ?: return null
        val body = post(
            "/v1/daemons",
            buildJsonObject { put("label", JsonPrimitive(label)) },
            bearer = token,
        )
        return body?.get("token")?.jsonPrimitive?.contentOrNull
    }

    /** Everything this account has registered, for the management screen. */
    suspend fun fetchRegistrations(): Registrations? {
        val token = accessToken() ?: return null
        val body = post("/v1/account", JsonObject(emptyMap()), bearer = token) ?: return null

        val devices = body["devices"]?.jsonArray?.map { element ->
            val item = element.jsonObject
            Registration(
                id = item.text("id"),
                label = item.text("label").ifEmpty { "Device" },
                detail = if (item.text("platform") == "fcm") "Android" else "Apple",
                version = item.textOrNull("version"),
                at = item["updatedAt"]?.jsonPrimitive?.doubleOrNull,
            )
        } ?: emptyList()

        // `machines` is the relay's own JSON key. It names a paired daemon —
        // a runner — and stays spelled that way because the relay's API is a
        // contract, not a word this app gets to choose.
        val runners = body["machines"]?.jsonArray?.map { element ->
            val item = element.jsonObject
            Registration(
                id = item.text("id"),
                label = item.text("label").ifEmpty { "Runner" },
                detail = "Paired",
                version = item.textOrNull("version"),
                at = item["lastSeenAt"]?.jsonPrimitive?.doubleOrNull
                    ?: item["createdAt"]?.jsonPrimitive?.doubleOrNull,
            )
        } ?: emptyList()

        return Registrations(devices, runners)
    }

    /**
     * Stop notifying a device, or stop a runner notifying anything.
     *
     * Revoking here rather than on the runner is the case that matters: a
     * laptop you no longer have is exactly the one you cannot run a command on.
     */
    suspend fun revoke(registration: Registration, kind: RegistrationKind): Boolean {
        val token = accessToken() ?: return false
        val path = if (kind == RegistrationKind.DEVICE) "/v1/devices/revoke" else "/v1/daemons/revoke"
        return post(
            path,
            buildJsonObject { put("id", JsonPrimitive(registration.id)) },
            bearer = token,
        ) != null
    }

    // MARK: - Plumbing

    private fun store(body: JsonObject) {
        body["accessToken"]?.jsonPrimitive?.contentOrNull?.let { tokens.write(KEY_ACCESS, it) }
        body["refreshToken"]?.jsonPrimitive?.contentOrNull?.let { tokens.write(KEY_REFRESH, it) }
        body["userId"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotEmpty() }?.let {
            _userId.value = it
            preferences.edit().putString(KEY_USER_ID, it).apply()
        }
        body["email"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotEmpty() }?.let {
            _email.value = it
            preferences.edit().putString(KEY_EMAIL, it).apply()
        }
    }

    private suspend fun post(
        path: String,
        body: JsonObject,
        bearer: String? = null,
    ): JsonObject? = withContext(Dispatchers.IO) {
        runCatching {
            // `relay` is a setting anyone can type into, so a stray space in it
            // must surface as a relay that would not answer rather than a
            // crash.
            val connection = (URL(relay + path).openConnection() as HttpURLConnection).apply {
                requestMethod = "POST"
                setRequestProperty("Content-Type", "application/json")
                bearer?.let { setRequestProperty("Authorization", "Bearer $it") }
                connectTimeout = 15_000
                readTimeout = 15_000
                doOutput = true
            }
            connection.outputStream.use { it.write(body.toString().toByteArray()) }
            if (connection.responseCode != 200) return@runCatching null
            val text = connection.inputStream.bufferedReader().readText()
            json.parseToJsonElement(text).jsonObject
        }.getOrNull()
    }

    /**
     * Read a JWT's `exp` without verifying it.
     *
     * Verification is the relay's job — it has the JWKS. This only decides
     * whether to bother sending a token that is already stale, and a forged
     * expiry buys nothing but an extra refresh.
     */
    private fun jwtExpiry(token: String): Long? {
        val parts = token.split(".")
        if (parts.size != 3) return null
        return runCatching {
            val payload = Base64.decode(parts[1], Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
            val claims = json.parseToJsonElement(String(payload)).jsonObject
            (claims["exp"]?.jsonPrimitive?.doubleOrNull ?: return null).toLong() * 1000
        }.getOrNull()
    }

    /**
     * Null rather than zeros if the system has no randomness for us.
     *
     * The iOS app's version of this discarded the status once, which meant the
     * verifier became a fixed publicly known constant, sign-in still appeared
     * to work, and PKCE silently protected nothing — which is the one thing
     * standing between a custom URL scheme any app can claim and account
     * takeover.
     */
    private fun randomVerifier(): String? = runCatching {
        val bytes = ByteArray(32)
        SecureRandom().nextBytes(bytes)
        Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
    }.getOrNull()

    private fun challengeFor(verifier: String): String? = runCatching {
        val digest = MessageDigest.getInstance("SHA-256").digest(verifier.toByteArray(Charsets.US_ASCII))
        Base64.encodeToString(digest, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
    }.getOrNull()

    companion object {
        /**
         * The relay this build's channel talks to.
         *
         * One relay per channel, the same partition the application id follows.
         * They are separate deployments with separate databases and separate
         * WorkOS environments, so signing in on the beta is a different account
         * from signing in on the release — which it already was, since the
         * WorkOS user id is the account id and each environment issues its own.
         *
         * Release's URL is unchanged and must stay that way: it is compiled
         * into store builds that cannot be told a new one on demand. Matches
         * the iOS derivation exactly.
         */
        val DEFAULT_RELAY: String
            get() = when (AppVersion.channel) {
                "stable" -> "https://relay.farcooler.com"
                "preview" -> "https://relay-preview.farcooler.com"
                "canary" -> "https://relay-canary.farcooler.com"
                // Anything unstamped is a local build — see AppVersion.channel,
                // which defaults the same way and for the same reason.
                else -> "https://relay-local.farcooler.com"
            }
        // The scheme this build actually registered, from the same value the
        // manifest's intent filter was written with — see build.gradle.kts.
        // Per channel, because Android picks between two apps claiming one
        // scheme and a sign-in delivered to the wrong channel fails there: the
        // code was issued against a different WorkOS environment.
        private val SCHEME = BuildConfig.AUTH_SCHEME
        private val REDIRECT_URI = "${BuildConfig.AUTH_SCHEME}://auth"

        private const val KEY_RELAY = "account.relay"
        private const val KEY_CLIENT_ID = "account.clientID"
        private const val KEY_USER_ID = "account.userId"
        private const val KEY_EMAIL = "account.email"

        /** Credentials, so: the Keystore. Labels stay in preferences. */
        const val KEY_ACCESS = "account.access"
        const val KEY_REFRESH = "account.refresh"
    }
}

/** One row on the management screen. */
data class Registration(
    val id: String,
    val label: String,
    val detail: String,
    /**
     * What this thing last reported running, or null if it never has.
     *
     * The point of the whole devices screen once there is more than one
     * runner: seeing which one is behind without opening each of them. Null is
     * not an error — a paired runner that has never had an agent get stuck has
     * never had a reason to talk to the relay.
     */
    val version: String?,
    /** Last heard from, as a Unix millisecond stamp. */
    val at: Double?,
)

data class Registrations(val devices: List<Registration>, val runners: List<Registration>)

enum class RegistrationKind { DEVICE, RUNNER }

private fun JsonObject.text(key: String): String =
    this[key]?.jsonPrimitive?.contentOrNull.orEmpty()

private fun JsonObject.textOrNull(key: String): String? =
    this[key]?.jsonPrimitive?.contentOrNull
