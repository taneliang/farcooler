package com.farcooler.account

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * This phone's Key A, registered with the team directory on sign-in and on
 * launch while signed in (ov-549, the same rule as `DeviceKeyRegistration` in
 * AgentKit).
 *
 * Two calls: `directory/devices/challenge`, then `directory/devices/register`
 * with the label, the challenge and what [sign] made of it. [sign] gets the
 * account this phone has signed in, never one the relay named, and answers the
 * core's JSON (`NativeClient.nativeDeviceRegistration`), so no key and no
 * statement is built in Kotlin.
 *
 * Nothing here reaches a screen. A phone that couldn't register works exactly
 * as before; what it misses is being listed to a team's admins.
 */
class DeviceKeyRegistration(
    /** One authenticated POST to the relay. */
    private val post: suspend (path: String, body: JsonObject) -> RelayAnswer,
    private val sign: (account: String, challenge: String) -> String?,
    private val label: () -> String,
) {
    /** What one attempt came to. */
    sealed interface Outcome {
        /** `trusted`, `pending` or `certified`. */
        data class Registered(val state: String) : Outcome
        /** The relay said no, or said something this can't read. */
        data class Refused(val status: Int) : Outcome
        data object Unreachable : Outcome
        /** The core wouldn't sign: no key, or a challenge in the wrong shape. */
        data object NoProof : Outcome
    }

    /** Guards the three fields below; never held across a relay call. */
    private val mutex = Mutex()
    private var registered: String? = null
    private var running = false
    /**
     * An account asked for while another registration was in flight: run once
     * that one ends, never dropped (review L5). The latest ask wins.
     */
    private var queued: String? = null

    /**
     * Register for [account], unless that already happened this launch.
     *
     * A call that arrives while another is in flight (a sign-in during the
     * launch's registration) is queued and run when that one ends, and answers
     * null itself; the call already running answers the last outcome.
     */
    suspend fun registerIfNeeded(account: String): Outcome? {
        if (account.isEmpty()) return null
        mutex.withLock {
            if (registered == account) return null
            if (running) {
                queued = account
                return null
            }
            running = true
        }
        var current = account
        while (true) {
            val outcome = runCatching { register(current) }.getOrElse { error ->
                mutex.withLock { running = false; queued = null }
                throw error
            }
            val next = mutex.withLock {
                if (outcome is Outcome.Registered) registered = current
                val asked = queued
                queued = null
                if (asked == null || asked == registered) {
                    running = false
                    null
                } else asked
            } ?: return outcome
            current = next
        }
    }

    private suspend fun register(account: String): Outcome {
        val challenge = when (val answer = post(CHALLENGE, JsonObject(emptyMap()))) {
            is RelayAnswer.Answered ->
                answer.body["challenge"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotEmpty() }
                    ?: return Outcome.Refused(200)
            is RelayAnswer.Refused -> return Outcome.Refused(answer.status)
            is RelayAnswer.Unreachable -> return Outcome.Unreachable
        }
        val proof = sign(account, challenge)?.let(::proofOf) ?: return Outcome.NoProof
        val body = buildJsonObject {
            put("label", JsonPrimitive(label().take(80)))
            put("challenge", JsonPrimitive(challenge))
            put("keyA", JsonPrimitive(proof.first))
            put("proofA", JsonPrimitive(proof.second))
        }
        return when (val answer = post(REGISTER, body)) {
            is RelayAnswer.Answered ->
                answer.body["state"]?.jsonPrimitive?.contentOrNull?.takeIf { it in STATES }
                    ?.let { Outcome.Registered(it) } ?: Outcome.Refused(200)
            is RelayAnswer.Refused -> Outcome.Refused(answer.status)
            is RelayAnswer.Unreachable -> Outcome.Unreachable
        }
    }

    companion object {
        const val CHALLENGE = "/v1/directory/devices/challenge"
        const val REGISTER = "/v1/directory/devices/register"
        private val STATES = setOf("trusted", "pending", "certified")

        /** The core's answer as `keyA` and `proofA`, or null for a refusal. */
        fun proofOf(answer: String): Pair<String, String>? = runCatching {
            val obj = Json.parseToJsonElement(answer).jsonObject
            val keyA = obj["keyA"]?.jsonPrimitive?.contentOrNull ?: return null
            val proofA = obj["proofA"]?.jsonPrimitive?.contentOrNull ?: return null
            if (!keyA.startsWith("ssh-ed25519 ") || proofA.isEmpty()) null else keyA to proofA
        }.getOrNull()
    }
}
