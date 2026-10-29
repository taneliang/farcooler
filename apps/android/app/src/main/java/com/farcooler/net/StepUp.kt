package com.farcooler.net

import com.farcooler.core.refusalWord
import com.farcooler.model.StepUpFailure
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put

/**
 * Step-up on an admin action (ov-856, §3 of the design): one biometric or screen-lock prompt per action, signing the
 * runner's challenge for that exact action. AgentKit's `StepUp`, in Kotlin.
 *
 * What this file owns: the order of the calls (the client core builds the bytes and the statement; the device key
 * signs it; the core sends it). The key is [StepUpSigning]'s; the bytes that are signed are the core's
 * (`crates/client/src/session/step_up.rs`), the same on every platform.
 */
interface StepUpCalling {
    suspend fun call(method: String, args: JsonObject): JsonObject

    /** The runner's `what` for a refusal (`step_up_needed`), else its word, else null. */
    fun refusalWord(error: Throwable): String?
}

/** A device key that signs with the person's own verification. */
interface StepUpSigning {
    /** The 65-byte uncompressed P-256 point, null before a key exists. */
    val publicKey: ByteArray?

    /** Make the key if this device has none, answering its public point. */
    fun makeKey(): ByteArray

    /** ECDSA over SHA-256 of [statement], DER encoded, after the person verifies; [reason] is what the prompt says. */
    suspend fun sign(statement: ByteArray, reason: String): ByteArray
}

class StepUp(
    private val core: StepUpCalling,
    private val signer: StepUpSigning,
    /** This device's own id on the runner (what its line names). */
    private val clientId: String,
    /** The runner's host key as this device pinned it, never as the runner said. */
    private val hostPin: String,
) {
    /**
     * [method] with [args] (the ones the plain call takes), asking for the person's verification when this device
     * holds a trusted key. A device without one keeps its old way until it registers (the transition); and a runner
     * that asks for a signature anyway (`step_up_needed`) gets one if there's a key to sign with.
     */
    suspend fun perform(method: String, args: JsonObject, reason: String, hasTrustedKey: Boolean): JsonObject {
        if (hasTrustedKey) return signed(method, args, reason)
        return try {
            core.call(method, args)
        } catch (e: Exception) {
            if (core.refusalWord(e) != "step_up_needed" || signer.publicKey == null) throw e
            signed(method, args, reason)
        }
    }

    private suspend fun signed(method: String, args: JsonObject, reason: String): JsonObject {
        val prepared = core.call(
            "step_up.begin",
            buildJsonObject {
                put("method", method)
                put("args", args)
                put("clientId", clientId)
                put("hostPin", hostPin)
            },
        )
        val token = (prepared["token"] as? JsonPrimitive)?.contentOrNull ?: throw StepUpFailure.KeyUnavailable
        val statement = (prepared["statement"] as? JsonPrimitive)?.contentOrNull?.let { java.util.Base64.getDecoder().decode(it) }
            ?: throw StepUpFailure.KeyUnavailable
        val signature = signer.sign(statement, reason)
        return core.call(
            "step_up.finish",
            buildJsonObject {
                put("token", token)
                put("signature", java.util.Base64.getEncoder().encodeToString(signature))
            },
        )
    }

    /**
     * Offer this device's key to the runner for approval: makes the key if need be, and answers the six digits to show
     * beside "Allow Admin Actions", which another device of the same person checks.
     */
    suspend fun register(): String {
        val key = signer.publicKey ?: signer.makeKey()
        val answer = core.call("step_up.propose", buildJsonObject { put("publicKey", java.util.Base64.getEncoder().encodeToString(key)) })
        return (answer["code"] as? JsonPrimitive)?.contentOrNull?.takeIf { it.length == 6 } ?: throw StepUpFailure.KeyUnavailable
    }
}

/** The client core, as step-up calls it. */
class CoreStepUpCalls(private val send: suspend (String, JsonObject) -> JsonObject) : StepUpCalling {
    override suspend fun call(method: String, args: JsonObject): JsonObject = send(method, args)

    override fun refusalWord(error: Throwable): String? =
        (error as? com.farcooler.core.CoreException)?.let { it.what ?: it.word } ?: error.refusalWord
}

/** A P-256 public key as the runner takes it. */
object P256 {
    /** The 65-byte uncompressed point (0x04 || X || Y) of a P-256 public key. */
    fun uncompressed(key: java.security.PublicKey): ByteArray {
        val point = (key as java.security.interfaces.ECPublicKey).w
        fun fixed(value: java.math.BigInteger): ByteArray {
            val raw = value.toByteArray().let { if (it.size > 1 && it[0] == 0.toByte()) it.copyOfRange(1, it.size) else it }
            return ByteArray(32 - raw.size) + raw
        }
        return byteArrayOf(4) + fixed(point.affineX) + fixed(point.affineY)
    }
}
