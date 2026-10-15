package com.farcooler.account

import android.util.Log
import kotlinx.coroutines.delay
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

/**
 * Certifying the device this phone scanned in the same-account ceremony
 * (ov-550, the same rule as `DeviceCertification` in AgentKit): this phone
 * signs, with its own Key A, a certificate for the Key A in the code it read,
 * and posts it to `directory/devices/certify`.
 *
 * **The key comes from the scanned code, never from the relay**, and the
 * account is the one signed in here, only when the code named the same one:
 * the core has already refused a code for another account at the scan, and
 * this checks again before signing. [sign] answers the core's JSON
 * (`NativeClient.nativeDeviceCertificate`), so no statement is built in Kotlin.
 *
 * The new device registers its key when it signs in, which can land a moment
 * after its code goes up, so a 404 is retried a few times. Nothing here
 * reaches a screen; the log names the outcome only.
 */
class DeviceCertification(
    /** One authenticated POST to the relay. */
    private val post: suspend (path: String, body: JsonObject) -> RelayAnswer,
    private val sign: (account: String, keyA: String) -> String?,
    /** The account signed in here, or "" for none. */
    private val signedIn: () -> String,
    private val waits: List<Duration> = listOf(2.seconds, 5.seconds, 15.seconds, 30.seconds, 60.seconds),
    private val pause: suspend (Duration) -> Unit = { delay(it) },
    private val log: (String) -> Unit = { Log.i("DeviceCertification", it) },
) {
    enum class Outcome { CERTIFIED, ALREADY_CONFIRMED, NOT_REGISTERED, REFUSED, NOT_SIGNED, FAILED }

    suspend fun certifyScanned(account: String, keyA: String): Outcome {
        val outcome = run(account, keyA)
        log("device certification: $outcome")
        return outcome
    }

    private suspend fun run(account: String, keyA: String): Outcome {
        if (account.isEmpty() || signedIn() != account) return Outcome.NOT_SIGNED
        val certificate = sign(account, keyA)?.let(::certificateOf) ?: return Outcome.NOT_SIGNED
        var outcome = send(keyA, certificate)
        for (wait in waits) {
            if (outcome != Outcome.NOT_REGISTERED) break
            pause(wait)
            // Signed out, or someone else signed in, while waiting.
            if (signedIn() != account) return Outcome.NOT_SIGNED
            outcome = send(keyA, certificate)
        }
        return outcome
    }

    private suspend fun send(keyA: String, certificate: String): Outcome {
        val body = buildJsonObject {
            put("keyA", JsonPrimitive(keyA))
            put("certificate", JsonPrimitive(certificate))
        }
        return when (val answer = post(CERTIFY, body)) {
            is RelayAnswer.Answered -> Outcome.CERTIFIED
            is RelayAnswer.Unreachable -> Outcome.FAILED
            is RelayAnswer.Refused -> when (answer.status) {
                404 -> Outcome.NOT_REGISTERED
                403 -> Outcome.REFUSED
                409 -> {
                    val state = answer.body?.get("state")?.jsonPrimitive?.contentOrNull
                    if (state == "trusted" || state == "certified") Outcome.ALREADY_CONFIRMED else Outcome.REFUSED
                }
                else -> Outcome.FAILED
            }
        }
    }

    companion object {
        const val CERTIFY = "/v1/directory/devices/certify"

        /** The core's answer's `certificate`, or null for a refusal. */
        fun certificateOf(answer: String): String? = runCatching {
            kotlinx.serialization.json.Json.parseToJsonElement(answer).jsonObject["certificate"]
                ?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotEmpty() }
        }.getOrNull()
    }
}
