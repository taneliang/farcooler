package com.farcooler.ceremony

import com.farcooler.account.RelayAnswer
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

// The reply, sealed for a browser that can't scan it (ov-770).
//
// A browser has no camera, so when the offer this phone scanned names a
// `reply_key`, the reply it built is also sealed to that key, posted to the
// relay this phone is signed in to, and a six-character code goes up beside
// the QR code for the person to match against the browser's.
//
// THE SEAL AND THE CODE ARE RUST'S (`crates/ceremony/src/seal.rs`), through
// `nativeCeremonySeal`. Nothing here encrypts, hashes or compares: this file
// hands over the two strings the ceremony already holds, posts what comes
// back, and keeps the code for the screen. An offer without a reply key never
// reaches the core here, so that ceremony is exactly the QR-only one it was.
//
// Nothing here is logged. `sealed` is the fleet's topology, encrypted.

/**
 * The sealed half of a reply this phone built: sealing it, posting it once,
 * and the code to show. Owned by [CeremonyStore], which hands it each reply.
 */
class SealedReply {
    enum class State {
        /** The offer asked for no sealed reply: the QR code is the whole reply. */
        NONE,
        SENDING,

        /** The relay holds it for the browser. */
        SENT,

        /** It couldn't be sealed or posted. The QR code is still on screen. */
        NOT_SENT,
    }

    /**
     * One POST to the relay this app is signed in to, through its session.
     * Not signed in until the screen wires it, so nothing is posted by default.
     */
    var post: suspend (ceremony: String, account: String, sealed: String) -> SealedReplyPost.Outcome =
        { _, _, _ -> SealedReplyPost.Outcome.NOT_SIGNED_IN }

    private val _state = MutableStateFlow(State.NONE)
    val state: StateFlow<State> = _state.asStateFlow()

    /**
     * The six characters both screens show, as Rust drew them from the seal's
     * exporter secret. Null until the relay has stored the reply, and null
     * after any failure: never up while it could still be ground against.
     */
    private val _code = MutableStateFlow<String?>(null)
    val code: StateFlow<String?> = _code.asStateFlow()

    /** Each granted runner and the host key it pins, one tap behind the code. */
    private val _hostKeys = MutableStateFlow<List<Pair<String, String>>>(emptyList())
    val hostKeys: StateFlow<List<Pair<String, String>>> = _hostKeys.asStateFlow()

    /**
     * Seal and post [replyJson], when [offerJson] asked for that.
     *
     * [offerJson] is the scan's answer, checked by the core a moment ago; its
     * `reply_key` is read here only to decide whether to ask the core at all.
     * Every refusal posts nothing: the core is the only judge of the key, the
     * freshness window and whether the reply answers this offer.
     */
    suspend fun send(core: CeremonyCalls, offerJson: String, replyJson: String, heldMs: Long) {
        reset()
        val asked = CeremonyCore.decode(offerJson, CeremonyOffer.serializer()) ?: return
        if (asked.replyKey == null) return
        val sealed = (core.seal(offerJson, replyJson, heldMs) as? CeremonyCore.Answer.Payload)
            ?.let { sealedOf(it.json) }
        if (sealed == null) {
            _state.value = State.NOT_SENT
            return
        }
        _hostKeys.value = CeremonyCore.decode(replyJson, CeremonyManifest.serializer())
            ?.runners.orEmpty().map { it.label to it.hostKey }
        _state.value = State.SENDING
        val outcome = post(asked.ceremony, asked.account, sealed.first)
        val posted = outcome == SealedReplyPost.Outcome.POSTED
        _code.value = if (posted) sealed.second else null
        _state.value = if (posted) State.SENT else State.NOT_SENT
    }

    fun reset() {
        _state.value = State.NONE
        _code.value = null
        _hostKeys.value = emptyList()
    }

    private fun sealedOf(answer: String): Pair<String, String>? = runCatching {
        val body = kotlinx.serialization.json.Json.parseToJsonElement(answer).jsonObject
        val sealed = body["sealed"]?.jsonPrimitive?.contentOrNull ?: return null
        val code = body["code"]?.jsonPrimitive?.contentOrNull ?: return null
        sealed to code
    }.getOrNull()
}

/** Posting a sealed reply to the relay's `POST /v1/ceremony/reply` (ov-770). */
object SealedReplyPost {
    enum class Outcome { POSTED, ALREADY_POSTED, REFUSED, NOT_SIGNED_IN, FAILED }

    const val PATH = "/v1/ceremony/reply"

    /**
     * The relay's three fields and nothing more: `ceremony` and `account` from
     * the scanned offer, and `sealed` as the core returned it. [post] is the
     * account's own authenticated POST, so this goes to the relay it's signed
     * in to.
     */
    suspend fun send(
        post: suspend (path: String, body: JsonObject) -> RelayAnswer,
        ceremony: String,
        account: String,
        sealed: String,
    ): Outcome {
        val body = buildJsonObject {
            put("ceremony", JsonPrimitive(ceremony))
            put("account", JsonPrimitive(account))
            put("sealed", JsonPrimitive(sealed))
        }
        return when (val answer = post(PATH, body)) {
            is RelayAnswer.Answered -> Outcome.POSTED
            is RelayAnswer.Unreachable ->
                if (answer.reason == "signed out") Outcome.NOT_SIGNED_IN else Outcome.FAILED
            is RelayAnswer.Refused -> when (answer.status) {
                409 -> Outcome.ALREADY_POSTED
                403 -> Outcome.REFUSED
                else -> Outcome.FAILED
            }
        }
    }
}
