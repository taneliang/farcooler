package com.farcooler.ceremony

import android.content.Context
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.CancellationSignal
import android.os.SystemClock
import android.util.Log
import com.farcooler.core.NativeClient
import com.farcooler.core.NativeLibrary
import com.farcooler.data.NodeIdentity
import com.farcooler.data.Reach
import com.farcooler.data.Runner
import com.farcooler.model.DirectRoute
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlin.coroutines.resume

// The enrollment ceremony, as this app takes part in it.
//
// EVERY RULE THAT DECIDES WHETHER A SCAN IS ACCEPTABLE IS IN RUST, behind the
// four entry points below — version, channel, freshness, the account, the three
// echoes, the byte budget. This file shows a code, points a camera at one, and
// turns the word that comes back into a sentence. If something here starts to
// look like validation, it belongs in `crates/ceremony/src/lib.rs` instead.
//
// A refusal crosses as a stable word, never a sentence: `{"error":"stale"}`. The
// copy for each is [Refusal], and it is the app's, so no Rust error string can
// reach a screen.

// MARK: - What the codes carry

/**
 * The first code: what a new device shows.
 *
 * Decoded only to draw the confirmation. Nothing is decided from these fields
 * here — the account they name was already checked by the scan that produced
 * them.
 */
@Serializable
data class CeremonyOffer(
    val v: Int = 0,
    @SerialName("key_a") val keyA: String = "",
    @SerialName("key_b") val keyB: String? = null,
    val name: String = "",
    val account: String = "",
    val channel: String = "",
    val ceremony: String = "",
    /**
     * That device's tailcat node public key: 43 characters of unpadded
     * base64-URL, or empty from a device that has none.
     *
     * Empty is not "unknown". It is a device that cannot be admitted to a
     * tunnel — a v=1 offer, or a phone whose own mint failed — and it enrolls as
     * direct, which is what `can_be_granted_a_tunnel` reads the same field to
     * decide on the other side of the exchange. The default covers a code with
     * no such field at all rather than failing the whole ceremony over one that
     * only decides whether a tunnel is offered.
     */
    @SerialName("node_key") val nodeKey: String = "",
    /** A browser's key to seal the reply to (ov-770), or null: [SealedReply]. */
    @SerialName("reply_key") val replyKey: String? = null,
)

/**
 * One runner in a reply: everything a device needs to reach it, and nothing it
 * needs to trust it with.
 */
@Serializable
data class CeremonyRunner(
    val id: String,
    val label: String,
    /**
     * The `~/.ssh/config` alias, which only the Mac writes — per runner rather
     * than per host, because two runners on one box would both want `Host box`.
     */
    val alias: String,
    val user: String,
    @SerialName("host_key") val hostKey: String,
    val reach: Reach,
    /**
     * This runner has not taken the key yet. Asleep, unreachable, or a client
     * core that cannot ask — either way access follows when a trusted device
     * next reaches it, and the new device simply retries.
     */
    val pending: Boolean,
    /**
     * A tunneled runner's address, kept as the route a person may choose when
     * the tunnel fails (ov-485). Never dialed on its own; see [DirectRoute].
     */
    val alternative: Reach? = null,
) {
    /**
     * This runner as something this app can connect to.
     *
     * **Not nullable any more, and that is the third gap closed.** It used to
     * answer null for anything that was not [Reach.Direct], because [Runner] was
     * an address and a port and a tunnel has neither — so a phone could be
     * GRANTED a tunneled runner and still not keep one, which is most of why no
     * ceremony had ever produced one. [Runner] carries the reach now, so the
     * reach travels across whole rather than being taken apart into fields that
     * only fit one of its two shapes.
     *
     * A tunneled runner here always has a node key to dial with: the core
     * refuses the whole reply with `no_tunnel` when this device named none, so a
     * reply this app has accepted cannot contain one it could not use.
     */
    fun asRunner(): Runner = Runner(
        label = label,
        reach = reach,
        user = user,
        // A host key that came across empty stays null, which is what makes
        // the first connection report the fingerprint instead of trusting it.
        fingerprint = hostKey.ifEmpty { null },
    )
}

/** The reply: the runners a trusted device granted, addressed to one ceremony. */
@Serializable
data class CeremonyManifest(
    val v: Int = 0,
    val ceremony: String = "",
    val account: String = "",
    val channel: String = "",
    val target: String = "",
    val runners: List<CeremonyRunner> = emptyList(),
    /**
     * Whose runners these are, on a reply from a teammate's runner admin
     * (ov-525): their email. Null on a same-account reply. Shown, never
     * trusted: the reply proves nothing about who made it.
     */
    @SerialName("granted_by") val grantedBy: String? = null,
) {
    /**
     * The email the reply says granted it, or null when it names none (a
     * same-account reply). Only another account's grant is asked about (ov-543).
     */
    val grantor: String? get() = grantedBy?.trim()?.takeIf { it.isNotEmpty() }
}

/**
 * What a device asks before joining a teammate's runners (ov-543): the phone's
 * words and the Mac's (`CeremonyStore.joinTitle` on iOS, `TeammateJoinCopy` on
 * the Mac).
 *
 * `granted_by` is unauthenticated, so the body says what "this code says" and
 * never states it as fact.
 */
object TeammateJoinCopy {
    /** A runner's label is one line in a sentence, cut in the middle past this. */
    const val LABEL_LIMIT = 28

    fun title(runners: List<CeremonyRunner>): String =
        if (runners.size == 1) {
            "Join your teammate’s runner “${shortened(runners.first().label)}”?"
        } else {
            "Join your teammate’s runners?"
        }

    fun detail(from: String): String =
        "This code says $from added this device to their runners. Work there runs on the Claude " +
            "account signed in on each runner, so its usage and limits are shared. Join only if " +
            "you were adding this device with them just now."

    /** [label] cut in the middle to at most [limit] characters. */
    fun shortened(label: String, limit: Int = LABEL_LIMIT): String {
        if (label.length <= limit || limit < 2) return label
        val keep = limit - 1
        val head = keep - keep / 2
        return label.take(head) + "…" + label.takeLast(keep / 2)
    }
}

// MARK: - Refusals

/**
 * Why a code was refused, and what a person is told about it.
 *
 * The cases are the FFI's stable words. THE APP OWNS THE SENTENCE: the core
 * answers `malformed`, and what belongs on a screen is "This isn't a Far Cooler
 * code" — never a `serde_json` message, and never advice about an sshd setting
 * that was not the problem.
 *
 * The sentences are the phone's and the Mac's, word for word — `Refusal.title`
 * and `Refusal.message` in `apps/ios/FarCooler/Ceremony/CeremonyStore.swift`,
 * and `Refusal.title`/`Refusal.detail` on the Mac. One person holds both
 * devices during this ceremony and reads the refusal on whichever one scanned,
 * so two spellings of one refusal is a bug only that person ever sees, and
 * only by being refused twice. These read as sentences on all three platforms,
 * so Android's own sentence case and Apple's agree here with nothing to
 * reconcile.
 */
sealed interface Refusal {
    data class Version(val made: Int) : Refusal

    data class Channel(val named: String) : Refusal

    data object Malformed : Refusal

    data object WrongCeremony : Refusal

    data object WrongAccount : Refusal

    data object WrongTarget : Refusal

    data object Stale : Refusal

    data object AlreadyTaken : Refusal

    data object TooLarge : Refusal

    /**
     * The reply granted a runner reachable only through the tunnel, and this
     * device named no node key a tunnel would admit. The granting side's bug,
     * and the core refuses the whole reply rather than hand this device a
     * runner it could only fail to connect to.
     */
    data object NoTunnel : Refusal

    /**
     * The core answered nothing readable at all. Not one of its words: this is
     * the app failing, and it says so without saying how.
     */
    data object Unknown : Refusal

    val title: String
        get() = when (this) {
            is Version ->
                if (made > 1) "This code requires a newer version of Far Cooler"
                else "This code is from an older version of Far Cooler"
            is Channel -> "This code is from a different version of Far Cooler"
            Malformed -> "This isn’t a Far Cooler code"
            WrongCeremony -> "This code is for a different request"
            WrongAccount -> "This code is for a different account"
            WrongTarget -> "This code is for another device"
            Stale -> "This code expired"
            AlreadyTaken -> "This code was already used"
            TooLarge -> "Too many runners selected"
            NoTunnel -> "This device can’t reach one of those runners"
            Unknown -> "Couldn’t add this device"
        }

    val message: String
        get() = when (this) {
            is Version ->
                if (made > 1) "Update Far Cooler on this device, then try again."
                else "Update Far Cooler on the other device, then try again."
            is Channel -> {
                val which =
                    if (named.isEmpty()) "a different version"
                    else "Far Cooler ${named.replaceFirstChar { it.uppercase() }}"
                "The other device is using $which. Both devices must use the same version."
            }
            Malformed -> "Scan the code shown in Far Cooler on the other device."
            WrongCeremony ->
                "Show a new code on this device, then scan the code on the other device."
            WrongAccount -> "Both devices must be signed in to the same account."
            WrongTarget ->
                "Show a new code on this device, then scan the code on the other device."
            Stale -> "Show a new code, then scan it within two minutes."
            AlreadyTaken -> "Show a new code, then try again."
            TooLarge -> "Select fewer runners. You can add the others later."
            NoTunnel ->
                "One of them is reachable only through a tunnel, and this device isn’t on " +
                    "one. Try again, choosing runners it can reach."
            Unknown -> "Try again."
        }

    companion object {
        /** The core's word, and the two details a sentence needs beside one. */
        fun of(code: String, version: Int?, channel: String?): Refusal = when (code) {
            "version" -> Version(version ?: 0)
            "channel" -> Channel(channel.orEmpty())
            "malformed" -> Malformed
            "wrong_ceremony" -> WrongCeremony
            "wrong_account" -> WrongAccount
            "wrong_target" -> WrongTarget
            "stale" -> Stale
            "already_taken" -> AlreadyTaken
            "too_large" -> TooLarge
            "no_tunnel" -> NoTunnel
            else -> Unknown
        }
    }
}

/**
 * The core's calls the trusted device's side makes, as an interface so a test
 * can answer them without the JNI library (ov-550 review L5). [CeremonyCore]
 * is the real one.
 */
interface CeremonyCalls {
    fun scan(encoded: String, expectingAccount: String, heldMs: Long): CeremonyCore.Answer
    fun reply(offerJson: String, runners: List<CeremonyRunner>): CeremonyCore.Answer
    fun fingerprint(publicKey: String): String?
    fun clientId(publicKey: String): String?

    /**
     * Seal a reply to its offer's reply key (ov-770). A fake that doesn't
     * answer refuses, so it posts nothing.
     */
    fun seal(offerJson: String, manifestJson: String, heldMs: Long): CeremonyCore.Answer =
        CeremonyCore.Answer.Refused(Refusal.Unknown)
}

// MARK: - The four entry points

/**
 * Kotlin's view of the ceremony core.
 *
 * Kotlin cannot call the C ABI, so these go through the JNI shims in
 * `crates/android/src/lib.rs`, which are themselves a translation layer over the
 * same C entry points the Mac and iOS apps use. One implementation of the rules,
 * three platforms.
 *
 * Every call answers either the payload it was asked for or `{"error":"…"}`, so
 * every call comes back as [Answer].
 */
object CeremonyCore : CeremonyCalls {
    sealed interface Answer {
        data class Payload(val json: String) : Answer

        data class Refused(val refusal: Refusal) : Answer
    }

    private val json = Json { ignoreUnknownKeys = true }

    /**
     * Leg one, the displaying side. `keyB` is null on a phone: there is no Zed
     * on a phone, so there is no second key.
     *
     * [nodeKey] is this device's node PUBLIC key — the half a runner writes into
     * its allowlist, never the half this device keeps. Null is a device that has
     * not minted one, which is the `v=1` shape the core has always accepted: it
     * can be granted direct runners and no tunneled ones. **Null is also what a
     * failed mint sends**, and never a placeholder.
     */
    fun offer(name: String, account: String, keyA: String, nodeKey: String?): Answer =
        answer { NativeClient.nativeCeremonyOffer(name, account, keyA, null, nodeKey) }

    /**
     * Leg one, the scanning side.
     *
     * [expectingAccount] is handed to Rust rather than compared here. That rule
     * — whether a stranger's device may be granted your fleet — was implemented
     * in Swift once, which meant two of three apps did not have it;
     * `ceremony::accept_offer` is where it lives now and `wrong_account` is what
     * it answers.
     */
    override fun scan(encoded: String, expectingAccount: String, heldMs: Long): Answer =
        answer { NativeClient.nativeCeremonyScan(encoded, expectingAccount, heldMs.coerceAtLeast(0)) }

    /**
     * Leg two, the trusted device's side.
     *
     * A budget of 0 takes the core's conservative default. This app does not
     * compute one of its own: ZXing will encode whatever fits a version-40 code,
     * which is more than that default, so the smaller number is the safe one and
     * the one that keeps three platforms agreeing about which manifests are too
     * big.
     */
    override fun reply(offerJson: String, runners: List<CeremonyRunner>): Answer {
        val encoded = runCatching {
            json.encodeToString(ListSerializer(CeremonyRunner.serializer()), runners)
        }.getOrNull() ?: return Answer.Refused(Refusal.Unknown)
        return answer { NativeClient.nativeCeremonyReply(offerJson, encoded, 0) }
    }

    /**
     * Leg two, the new device's side. [alreadyTaken] is this device's own record
     * that the ceremony has answered once — one reply per ceremony, so a forged
     * one cannot follow a real one.
     */
    fun accept(
        encoded: String,
        expecting: String,
        alreadyTaken: Boolean,
        heldMs: Long,
    ): Answer = answer {
        NativeClient.nativeCeremonyAccept(
            encoded,
            expecting,
            alreadyTaken,
            heldMs.coerceAtLeast(0),
        )
    }

    /**
     * The fingerprint of a public key, as a person reads it off a screen.
     *
     * Computed in Rust, by `ssh-key`, which is what the daemon's fence and the
     * host-key check already use — this app implements no cryptography. It is
     * display only: the check that matters compares a reply's `target` against a
     * fingerprint Rust computes for itself.
     */
    override fun fingerprint(publicKey: String): String? {
        if (!NativeLibrary.loaded || publicKey.isEmpty()) return null
        return runCatching { NativeClient.nativeFingerprint(publicKey) }
            .getOrNull()
            ?.takeIf { it.startsWith("SHA256:") }
    }

    /**
     * The client id a device is enrolled under, derived from its own key.
     *
     * Derived in Rust, and derived rather than invented for the reason
     * `client_id` gives in `crates/ceremony/src/lib.rs`: an id that comes out
     * of the key is stable, so a device re-running a ceremony against a runner
     * it is already on lands on the id already in that runner's fence instead of
     * enrolling a second line naming the same device. The daemon's "this device
     * is already enrolled" arm compares client ids, so an id this app made up
     * would defeat that arm while looking, from here, like it had worked.
     *
     * Null when there is no core for this ABI or the text is not a public key.
     * There is deliberately no fallback: a locally minted id is precisely the
     * bug this call exists to stop, and one substituted here would reintroduce
     * it silently.
     */
    override fun clientId(publicKey: String): String? {
        if (!NativeLibrary.loaded || publicKey.isEmpty()) return null
        return runCatching { NativeClient.nativeClientId(publicKey) }
            .getOrNull()
            ?.takeIf { it.isNotEmpty() }
    }

    /**
     * Seal the reply built for a browser's offer to the key it named, with
     * the comparison code: `{"sealed","code"}` (ov-770). [heldMs] is judged
     * in Rust, so an offer read past the window is `stale` and nothing is
     * sealed.
     */
    override fun seal(offerJson: String, manifestJson: String, heldMs: Long): Answer =
        answer { NativeClient.nativeCeremonySeal(offerJson, manifestJson, heldMs.coerceAtLeast(0)) }

    /** A sealed reply, taken with the reply key's secret half: the new device's side. */
    fun takeSealed(sealed: String, secret: String, expecting: String): Answer =
        answer { NativeClient.nativeCeremonyTakeSealed(sealed, secret, expecting, false, 0) }

    /** A reply's comparison code, as Rust computes it, or null. */
    fun code(manifestJson: String): String? {
        if (!NativeLibrary.loaded) return null
        return runCatching { NativeClient.nativeCeremonyCode(manifestJson) }.getOrNull()
    }

    /**
     * A shim's answer, as a payload or a refusal.
     *
     * Null means the shim itself could not answer — no core for this ABI, or a
     * buffer that could not be sized — and that is [Refusal.Unknown]: the app
     * failing rather than a code being wrong.
     */
    private inline fun answer(call: () -> String?): Answer {
        if (!NativeLibrary.loaded) return Answer.Refused(Refusal.Unknown)
        val text = runCatching { call() }.getOrNull() ?: return Answer.Refused(Refusal.Unknown)
        val parsed = runCatching { json.parseToJsonElement(text).jsonObject }.getOrNull()
            ?: return Answer.Refused(Refusal.Unknown)
        // An `error` key is the only thing that distinguishes a refusal from a
        // payload, and it is the core's own word — never a sentence.
        val code = parsed["error"]?.jsonPrimitive?.contentOrNull
            ?: return Answer.Payload(text)
        return Answer.Refused(
            Refusal.of(
                code,
                parsed["version"]?.jsonPrimitive?.intOrNull,
                parsed["channel"]?.jsonPrimitive?.contentOrNull,
            )
        )
    }

    internal fun <T> decode(payload: String, of: kotlinx.serialization.KSerializer<T>): T? =
        runCatching { json.decodeFromString(of, payload) }.getOrNull()
}

// MARK: - The gate

/**
 * A fingerprint, at the moment of the tap.
 *
 * This is what someone standing at an unlocked phone runs into, and it is the
 * only thing between them and an enrolled device. An account check does not help
 * there: that phone is signed in, and they would be using its session.
 *
 * The platform's own `BiometricPrompt` rather than `androidx.biometric`, because
 * `minSdk = 37` makes the compatibility wrapper weight for API levels this app
 * does not build for. `BIOMETRIC_STRONG or DEVICE_CREDENTIAL` is what makes the
 * fallback the device's PIN, pattern or password and never nothing — a phone
 * with no biometrics enrolled still has to answer something. A cancelled or
 * failed authentication enrolls nothing.
 */
object ConfirmingTap {
    enum class Outcome {
        CONFIRMED,

        /** Someone changed their mind. Not a failure, and not worth a screen. */
        CANCELLED,

        /** It asked and did not get an answer it accepted, or it could not ask. */
        REFUSED,
    }

    private const val ALLOWED = BiometricManager.Authenticators.BIOMETRIC_STRONG or
        BiometricManager.Authenticators.DEVICE_CREDENTIAL

    suspend fun ask(context: Context, title: String, subtitle: String): Outcome =
        suspendCancellableCoroutine { continuation ->
            val manager = context.getSystemService(BiometricManager::class.java)
            if (manager == null ||
                manager.canAuthenticate(ALLOWED) != BiometricManager.BIOMETRIC_SUCCESS
            ) {
                // No biometric and no screen lock at all. Nothing to fall back
                // to, so nothing is enrolled — the alternative is an unguarded
                // tap, which is the whole thing this gate exists to prevent.
                continuation.resume(Outcome.REFUSED)
                return@suspendCancellableCoroutine
            }

            val cancel = CancellationSignal()
            continuation.invokeOnCancellation { cancel.cancel() }

            val prompt = BiometricPrompt.Builder(context)
                .setTitle(title)
                .setSubtitle(subtitle)
                .setAllowedAuthenticators(ALLOWED)
                // The explicit confirmation after a passive match. A face
                // unlocking as the phone is raised is not somebody agreeing to
                // enroll a device.
                .setConfirmationRequired(true)
                .build()

            prompt.authenticate(
                cancel,
                context.mainExecutor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(
                        result: BiometricPrompt.AuthenticationResult
                    ) {
                        if (continuation.isActive) continuation.resume(Outcome.CONFIRMED)
                    }

                    override fun onAuthenticationError(code: Int, message: CharSequence) {
                        // There is no negative-button case to consider: allowing
                        // DEVICE_CREDENTIAL means the prompt has no negative
                        // button, and `setNegativeButton` alongside it throws.
                        val changedTheirMind =
                            code == BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED ||
                                code == BiometricPrompt.BIOMETRIC_ERROR_CANCELED
                        if (continuation.isActive) {
                            continuation.resume(
                                if (changedTheirMind) Outcome.CANCELLED else Outcome.REFUSED
                            )
                        }
                    }

                    // `onAuthenticationFailed` is a finger that did not match,
                    // not an outcome: the prompt is still up, and it will either
                    // succeed or error.
                },
            )
        }
}

// MARK: - Enrolling

/**
 * Putting a device's key into `~/.ssh/authorized_keys` on a runner.
 *
 * **The daemon owns the write.** Not a shell command appending a line, and
 * emphatically not anything in Kotlin: that file is the one whose corruption
 * costs somebody SSH access to their own machine, so the write is
 * descriptor-anchored, `O_NOFOLLOW`, locked, atomic and `fsync`ed twice in
 * `crates/daemon/src/enrollment.rs`. The app's part is to ask.
 */
fun interface Enroller {
    /**
     * Enroll [publicKey] on each runner, and answer with the ones it WAS
     * written to, each with the tunnel token it answered (empty when none).
     * A runner missing from the answer did not take the key.
     *
     * Those are not a failure of the ceremony: a runner that was asleep is an
     * ordinary outcome the manifest already carries, as `pending`.
     *
     * [nodeKey] is the new device's node public key, off its offer, and it is
     * what lets the runner's tunnel admit that device later. It goes to every
     * runner in [runners], because each of them writes its own line.
     */
    suspend fun enroll(
        publicKey: String,
        label: String,
        clientId: String,
        nodeKey: String,
        runners: List<CeremonyRunner>,
    ): Map<String, String>
}

// MARK: - The state machine

/** One runner, as a row someone can tick. */
data class RunnerRow(val runner: Runner, val picked: Boolean) {
    val label: String get() = runner.displayLabel

    /**
     * The second line. Which account a runner is reached through is not
     * something this device records, so what is shown is what it does know:
     * where the runner is and who it logs in as.
     */
    val detail: String get() = runner.reach.detail(user = runner.user)
}

/**
 * The sentence naming the runners [reply] carries as an address though
 * [granting] meant Tailcat, or null when none did.
 *
 * A reply that doesn't decode names nobody, so a fallback in it would go
 * unnamed. It comes from our own core and should never happen; when it does,
 * [onUndecodable] is told the exception's class name, never its message: a
 * decoding error quotes the input, and the reply holds tunnel tokens (ov-530 R2-4, M1).
 */
internal fun addressOnlyNoteFor(
    granting: List<CeremonyRunner>,
    reply: String,
    onUndecodable: (String) -> Unit,
): String? {
    val manifest = try {
        kotlinx.serialization.json.Json { ignoreUnknownKeys = true }
            .decodeFromString(CeremonyManifest.serializer(), reply)
    } catch (e: IllegalArgumentException) {
        // SerializationException is one.
        onUndecodable(e::class.java.simpleName)
        return null
    }
    val ids = DirectRoute.addressOnly(
        granting.map { it.id to it.reach }, manifest.runners.map { it.id to it.reach },
    )
    return DirectRoute.addressOnlySentence(granting.filter { it.id in ids }.map { it.label })
}

/**
 * The ceremony, from this app's side of it — either side.
 *
 * One store for both because they are one exchange: the device being added shows
 * a code and takes a reply; the device already trusted reads a code and gives
 * one back. Which methods a screen calls is what makes it one or the other.
 */
class CeremonyStore(
    /** The opaque relay account id both devices must share. */
    val account: String,
    /** What a person calls that account. Copy only. */
    val accountEmail: String,
    val deviceName: String,
) {
    sealed interface Phase {
        data object Idle : Phase

        /** The new device's own code is on screen. */
        data object ShowingOffer : Phase

        data object Scanning : Phase

        /**
         * The scanned code belongs to another account. Its own phase, and it
         * appears BEFORE the runner list — the runners are never on screen with
         * only a fingerprint between a stranger and them.
         */
        data object Mismatch : Phase

        data object Confirming : Phase

        /** The tap was confirmed; the keys are going out and the reply is next. */
        data object Enrolling : Phase

        /** The reply is on screen for the new device to scan. */
        data object ShowingManifest : Phase

        data object Done : Phase

        /**
         * A reply from another account's runner admin was taken and is held
         * until the person says Join (ov-543). Nothing is added before that.
         */
        data object ConfirmingJoin : Phase

        data class Refused(val refusal: Refusal) : Phase
    }

    /**
     * Vouch for the device just added to this phone's own account (ov-550):
     * a certificate for the Key A its code carried, signed by this phone and
     * posted to the directory. The app runs it outside this screen's life.
     */
    var certify: (account: String, keyA: String) -> Unit = { _, _ -> }

    /** The core, for reading a code and building the reply. A test answers it. */
    internal var core: CeremonyCalls = CeremonyCore

    /** The reply sealed for a browser, when its offer asked (ov-770). */
    val sealedReply = SealedReply()

    /** This device's elapsed clock, for the freshness window. A test answers it. */
    internal var clock: () -> Long = { SystemClock.elapsedRealtime() }

    private val _phase = MutableStateFlow<Phase>(Phase.Idle)
    val phase: StateFlow<Phase> = _phase.asStateFlow()

    /**
     * The runners on offer, and which are ticked. Beside [phase] rather than
     * inside it so a tick is one row changing rather than the whole screen's
     * state being replaced.
     */
    private val _rows = MutableStateFlow<List<RunnerRow>>(emptyList())
    val rows: StateFlow<List<RunnerRow>> = _rows.asStateFlow()

    /** What to draw as a code: this device's offer, or the reply it built. */
    private val _code = MutableStateFlow("")
    val code: StateFlow<String> = _code.asStateFlow()

    /**
     * The fingerprint of this device's own key, shown under its code so the
     * person adding it can compare it with their confirmation (ov-525).
     */
    private val _ownFingerprint = MutableStateFlow<String?>(null)
    val ownFingerprint: StateFlow<String?> = _ownFingerprint.asStateFlow()

    /** The device asking to be added, once its code has been read. */
    private val _offer = MutableStateFlow<CeremonyOffer?>(null)
    val offer: StateFlow<CeremonyOffer?> = _offer.asStateFlow()

    /** The fingerprint of the key that device is offering, as Rust computed it. */
    private val _fingerprint = MutableStateFlow<String?>(null)
    val fingerprint: StateFlow<String?> = _fingerprint.asStateFlow()

    /** The runners a reply granted, for the device that took it. */
    private val _granted = MutableStateFlow<List<CeremonyRunner>>(emptyList())
    val granted: StateFlow<List<CeremonyRunner>> = _granted.asStateFlow()

    /** The email a held teammate reply says granted it, while [Phase.ConfirmingJoin]. */
    private val _grantedBy = MutableStateFlow<String?>(null)
    val grantedBy: StateFlow<String?> = _grantedBy.asStateFlow()

    /**
     * The gate said no. Not a refusal — nothing was scanned wrong and no rule
     * was broken — so it is a dialog over the confirmation rather than a screen
     * that replaces it.
     */
    private val _declined = MutableStateFlow(false)
    val declined: StateFlow<Boolean> = _declined.asStateFlow()

    /**
     * Some picked runners did not take the key. Reported, without a cause: from
     * here the cause is not knowable — asleep, no daemon, a damaged fence and a
     * client core that cannot ask all look identical — and a screen that guesses
     * is how an app ends up telling somebody to loosen an sshd setting that was
     * never the problem.
     */
    private val _someRunnersPending = MutableStateFlow(false)
    val someRunnersPending: StateFlow<Boolean> = _someRunnersPending.asStateFlow()

    /**
     * The offer this device is showing, exactly as the core returned it: what
     * goes in the code and what is passed back as `expecting` are the same
     * string, so what it shows and what it remembers cannot drift apart.
     */
    private var showing: String = ""
    private var scannedCode: String = ""

    /**
     * When this device's own code went up, and when it read someone else's.
     *
     * [SystemClock.elapsedRealtime] rather than wall time: the freshness window
     * is a duration, and a clock that can be set — or that a time zone can move
     * — is not what "two minutes ago" means. Both are this device's own reading,
     * which is the only one that counts.
     */
    private var showingSince: Long? = null
    private var scannedAt: Long? = null

    /**
     * One reply per ceremony, recorded here because "have I already taken one"
     * is state on the device rather than anything a code can say.
     */
    private var alreadyTaken = false

    // MARK: The device being added

    /**
     * Build and show this device's code.
     *
     * The node key is minted HERE, at the moment a device asks to be added,
     * because that is the only moment its public half has anywhere to go: the
     * offer carries it, the granting device writes it into a runner's allowlist,
     * and a tunneled runner can be granted back. Minted once and kept — showing
     * a second code, or enrolling a second runner, offers the same key, because
     * the node key is this DEVICE's identity rather than anything about a
     * runner.
     *
     * **A mint that fails does not stop an enrolment.** [NodeIdentity] answers
     * null on any build with no tunnel linked, and null here is the `v=1` offer,
     * which grants direct runners exactly as it always has. The missing SSH key
     * above is the only thing that refuses, because without one there is nothing
     * to enroll at all.
     */
    fun showOffer(publicKey: String?) {
        if (publicKey.isNullOrEmpty()) {
            _phase.value = Phase.Refused(Refusal.Unknown)
            return
        }
        alreadyTaken = false
        _someRunnersPending.value = false
        val answer = CeremonyCore.offer(
            deviceName, account, publicKey, NodeIdentity.offeredPublicKey,
        )
        when (answer) {
            is CeremonyCore.Answer.Refused -> _phase.value = Phase.Refused(answer.refusal)
            is CeremonyCore.Answer.Payload -> {
                showing = answer.json
                _code.value = answer.json
                _ownFingerprint.value = CeremonyCore.fingerprint(publicKey)
                showingSince = SystemClock.elapsedRealtime()
                _phase.value = Phase.ShowingOffer
            }
        }
    }

    /**
     * Take the reply this device scanned, or refuse it.
     *
     * `heldMs` is measured FROM THE MOMENT THIS DEVICE'S OWN CODE WENT UP, not
     * from the instant of the scan. Measured from the scan it would always be
     * near zero and the freshness rule would do nothing; measured from the code
     * going up it bounds the thing that actually needs bounding — a code left on
     * a screen for an hour, answered by a reply that arrives now.
     */
    fun takeReply(encoded: String) {
        val heldMs = showingSince?.let { SystemClock.elapsedRealtime() - it } ?: Long.MAX_VALUE
        when (val answer = CeremonyCore.accept(encoded, showing, alreadyTaken, heldMs)) {
            // Refused without being consumed — the core takes nothing it did not
            // accept — so showing a fresh code and trying again is the whole
            // recovery.
            is CeremonyCore.Answer.Refused -> _phase.value = Phase.Refused(answer.refusal)
            is CeremonyCore.Answer.Payload -> {
                val manifest =
                    CeremonyCore.decode(answer.json, CeremonyManifest.serializer())
                if (manifest == null) {
                    _phase.value = Phase.Refused(Refusal.Unknown)
                    return
                }
                alreadyTaken = true
                receive(manifest)
            }
        }
    }

    /**
     * What an accepted reply leads to. Another account's runners are asked
     * about, never joined silently (ov-543): they wait in [granted] until
     * [join], and [cancelJoin] drops them. Your own devices' replies are done.
     */
    internal fun receive(manifest: CeremonyManifest) {
        _someRunnersPending.value = manifest.runners.any { it.pending }
        val from = manifest.grantor
        if (from != null) {
            _granted.value = manifest.runners
            _grantedBy.value = from
            _phase.value = Phase.ConfirmingJoin
            return
        }
        _granted.value = manifest.runners
        _phase.value = Phase.Done
    }

    /** The person said yes to a teammate's runners: they're this device's now. */
    fun join() {
        if (_phase.value != Phase.ConfirmingJoin) return
        _phase.value = Phase.Done
    }

    /**
     * The person said no. Nothing was added; the reply stays consumed, so a new
     * code is the way back.
     */
    fun cancelJoin() {
        if (_phase.value != Phase.ConfirmingJoin) return
        _granted.value = emptyList()
        _grantedBy.value = null
        _someRunnersPending.value = false
        _phase.value = Phase.Idle
    }

    /**
     * Back to the code already on screen, without minting another.
     *
     * Cancelling a scan must not change the code: the other device may have read
     * it a moment ago and be picking runners for exactly that ceremony, and a
     * fresh id here would turn their reply into a refusal.
     */
    fun showCodeAgain() {
        if (showing.isEmpty()) return
        _code.value = showing
        _phase.value = Phase.ShowingOffer
    }

    // MARK: The device already trusted

    fun beginScanning() {
        _offer.value = null
        _fingerprint.value = null
        _phase.value = Phase.Scanning
    }

    /**
     * Read a scanned code, and let the answer decide which screen it leads to.
     *
     * No comparison happens here. The account is passed into the core, and
     * `wrong_account` coming back is what routes to [Phase.Mismatch] — which is
     * a screen and not a step: the runner list is not behind it.
     */
    fun read(encoded: String, runners: List<Runner>, grantingFrom: Runner?) {
        val now = clock()
        // Zero, because this is the scan. The elapsed reading that does work is
        // taken again at the confirmation, below.
        when (val answer = core.scan(encoded, account, 0)) {
            is CeremonyCore.Answer.Refused ->
                _phase.value =
                    if (answer.refusal == Refusal.WrongAccount) Phase.Mismatch
                    else Phase.Refused(answer.refusal)

            is CeremonyCore.Answer.Payload -> {
                val decoded = CeremonyCore.decode(answer.json, CeremonyOffer.serializer())
                if (decoded == null) {
                    _phase.value = Phase.Refused(Refusal.Unknown)
                    return
                }
                scannedCode = encoded
                scannedAt = now
                _offer.value = decoded
                _fingerprint.value = core.fingerprint(decoded.keyA)
                // Only the runner being granted from is ticked. Everything else
                // is listed and unticked: granting more than was asked for is
                // not something a default gets to do.
                _rows.value = runners.map { RunnerRow(it, picked = it.id == grantingFrom?.id) }
                _phase.value = Phase.Confirming
            }
        }
    }

    fun toggle(runnerId: String) {
        _rows.value = _rows.value.map {
            if (it.runner.id == runnerId) it.copy(picked = !it.picked) else it
        }
    }

    fun dismissDeclined() {
        _declined.value = false
    }

    /**
     * Confirm the grant: the gate, then the freshness check, then the keys, then
     * the reply.
     *
     * The order is the argument. Nothing is written before the gate answers, and
     * the freshness check happens HERE rather than only at the scan — a
     * confirmation left open past the window has to refuse rather than enroll,
     * and this second call is what makes it.
     */
    suspend fun confirm(
        gate: suspend (title: String, subtitle: String) -> ConfirmingTap.Outcome,
        enroller: Enroller,
    ) {
        val asking = _offer.value
        if (asking == null || scannedCode.isEmpty()) {
            _phase.value = Phase.Refused(Refusal.Unknown)
            return
        }

        val outcome = gate(
            "Add “${asking.name}”?",
            "Confirm adding ${asking.name} to your runners",
        )
        when (outcome) {
            // Changing your mind is not a failure and gets no screen.
            ConfirmingTap.Outcome.CANCELLED -> return
            ConfirmingTap.Outcome.REFUSED -> {
                _declined.value = true
                return
            }
            ConfirmingTap.Outcome.CONFIRMED -> Unit
        }

        _phase.value = Phase.Enrolling

        val heldMs = scannedAt?.let { clock() - it } ?: Long.MAX_VALUE
        val rescanned = when (val answer = core.scan(scannedCode, account, heldMs)) {
            is CeremonyCore.Answer.Refused -> {
                _phase.value =
                    if (answer.refusal == Refusal.WrongAccount) Phase.Mismatch
                    else Phase.Refused(answer.refusal)
                return
            }
            is CeremonyCore.Answer.Payload -> answer.json
        }

        // Asked for before the reply is built, so `pending` is a fact rather
        // than a hope: a runner in that code is claimed as granted, and a claim
        // no line was written for would be the one lie in this flow a person
        // could not detect.
        val wanted = picked()
        // Measured BEFORE anything is written (ov-485 H1): the core degrades a
        // reply over budget down to the address-only one, which no token
        // changes, so its refusal now is the only one the reply below can give.
        val measured = core.reply(rescanned, wanted)
        if (measured is CeremonyCore.Answer.Refused) {
            _phase.value = Phase.Refused(measured.refusal)
            return
        }
        // One id per device, and it is what the forced command will carry — so
        // it is what closing this device's sessions later will name. Derived in
        // Rust from the key in the code rather than made here: the ceremony
        // correlates by its own id, which is a different thing with a different
        // lifetime, and an id minted on this side would be a new one every time
        // this device enrolled — a second line in the fence naming a device
        // already in it. See [CeremonyCore.clientId].
        //
        // No id means nothing to enroll under, so nothing is asked for and every
        // picked runner travels `pending` — a true statement about the file on
        // each of them, and the same answer the Mac and iOS give. A UUID here
        // would not be a fallback, it would be the bug.
        val clientId = core.clientId(asking.keyA)
        val written = if (clientId == null) {
            emptyMap()
        } else {
            enroller.enroll(
                publicKey = asking.keyA,
                label = asking.name,
                clientId = clientId,
                // Straight off the offer. Empty for a v=1 device and for one
                // whose own mint failed, both of which enroll exactly as they
                // always have — and the ONLY route by which that key reaches
                // the runner's `authorized_keys` line, which is what a tunnel
                // checks before it admits anybody.
                nodeKey = asking.nodeKey,
                runners = wanted,
            )
        }
        _someRunnersPending.value = wanted.any { it.id !in written }
        // Tunnel first (ov-485): a runner that answered with a token is granted
        // through it, its address kept beside it. The rule is [DirectRoute.grant].
        val granting = wanted.map { runner ->
            val (reach, beside) = DirectRoute.grant(
                runner.reach, runner.alternative, written[runner.id].orEmpty(), asking.nodeKey.isNotEmpty(),
            )
            runner.copy(pending = runner.id !in written, reach = reach, alternative = beside)
        }

        when (val answer = core.reply(rescanned, granting)) {
            is CeremonyCore.Answer.Refused -> _phase.value = Phase.Refused(answer.refusal)
            is CeremonyCore.Answer.Payload -> {
                _code.value = answer.json
                _addressOnlyNote.value = addressOnlyNote(granting, answer.json)
                _phase.value = Phase.ShowingManifest
                // Your own device, confirmed just now: vouch for it in the
                // directory too, with the key from the code this phone read.
                certify(asking.account, asking.keyA)
                // A browser's offer: sealed to its key and posted too (ov-770).
                sealedReply.send(core, rescanned, answer.json, scannedAt?.let { clock() - it } ?: Long.MAX_VALUE)
            }
        }
    }

    /**
     * The runners the reply had to grant at their address alone, by name, or
     * null (ov-485 N1): the new device can't use Tailcat for them.
     */
    private val _addressOnlyNote = MutableStateFlow<String?>(null)
    val addressOnlyNote: StateFlow<String?> = _addressOnlyNote.asStateFlow()

    private fun addressOnlyNote(granting: List<CeremonyRunner>, reply: String): String? =
        addressOnlyNoteFor(granting, reply) { Log.w("CeremonyStore", "the ceremony reply didn't decode: $it") }

    /** The ticked runners, as the reply's records. */
    private fun picked(): List<CeremonyRunner> = _rows.value.filter { it.picked }.map { row ->
        CeremonyRunner(
            id = row.runner.id,
            label = row.runner.displayLabel,
            // The `~/.ssh/config` alias, which the Mac writes. Derived from the
            // label here; the Rust writer owns collisions and suffixes.
            alias = alias(row.runner.displayLabel),
            user = row.runner.user,
            // The host key this device pinned, or nothing when it never has.
            // Empty travels honestly: the new device then meets the ordinary
            // first-contact screen and a person looks at a fingerprint, rather
            // than being handed a pin nobody verified.
            hostKey = row.runner.fingerprint.orEmpty(),
            // Carried across as it stands rather than rebuilt from an address,
            // because this app's own list can now hold a tunneled runner and
            // rebuilding one would have granted a Direct with an empty host —
            // an entry that looks filled in and reaches nothing. Whether the
            // device on the other side may HAVE it is the core's decision, not
            // this screen's: it refuses the whole reply with `no_tunnel` when
            // that device named no node key.
            //
            // Whether it goes as the tunnel is [confirm]'s, once the runner
            // has answered: tunnel first when it answered with a token, the
            // address kept beside it (ov-485, [DirectRoute.grant]).
            reach = row.runner.reach,
            // Corrected once the enrollment above has answered.
            pending = true,
            // The other route this phone keeps for it (ov-485).
            alternative = alternatives[row.runner.id],
        )
    }

    /** This phone's other route for each runner, by id, set by the screen (ov-485). */
    var alternatives: Map<String, Reach> = emptyMap()

    private fun alias(label: String): String {
        val slug = label.lowercase().map { character ->
            if (character.isLetterOrDigit() || character == '-' || character == '.') character
            else '-'
        }.joinToString("").trim('-')
        return slug.ifEmpty { "runner" }
    }

    fun reset() {
        _phase.value = Phase.Idle
        _offer.value = null
        _fingerprint.value = null
        _rows.value = emptyList()
        _granted.value = emptyList()
        _declined.value = false
        _someRunnersPending.value = false
        _addressOnlyNote.value = null
        scannedCode = ""
        _code.value = ""
        sealedReply.reset()
    }

    companion object {
        /**
         * What this device calls itself in the code it shows.
         *
         * `Build.MODEL`, because Android stopped handing out the name a person
         * gave their phone without a permission — and asking for one to put a
         * nicer word on someone else's screen is not a trade worth making. It is
         * read by a human on the trusted device, so it is the readable form
         * rather than [com.farcooler.data.Identity.deviceName], which is the
         * slug that goes in a key's comment.
         */
        fun thisDeviceName(): String =
            android.os.Build.MODEL?.takeIf { it.isNotBlank() } ?: "Android device"

        /**
         * `SHA256:` and the first and last eight characters: the ends, which is
         * what a person compares across a desk, and none of the middle, which
         * nobody does. About 96 bits, where four and three were about 42, which
         * a patient attacker can grind a key to match (ov-525). The same form
         * as AgentKit's `KeyFingerprint.short`, so an iPhone or a Mac and this
         * phone can be compared at a glance.
         */
        fun abbreviated(fingerprint: String): String {
            if (!fingerprint.startsWith("SHA256:")) return fingerprint
            val body = fingerprint.removePrefix("SHA256:")
            if (body.length <= 20) return fingerprint
            return "SHA256:${body.take(8)}…${body.takeLast(8)}"
        }
    }
}
