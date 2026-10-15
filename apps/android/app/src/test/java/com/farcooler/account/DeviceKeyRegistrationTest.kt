package com.farcooler.account

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * This phone's Key A registered on sign-in (ov-549): the two calls, what is
 * signed and for whom, and each answer. The relay and the core are faked; the
 * signature itself is proved in Rust (`device_keys_tests.rs`).
 */
class DeviceKeyRegistrationTest {
    private val challenge = "ab".repeat(32)
    private val proof = """{"keyA":"ssh-ed25519 AAAAkey","proofA":"-----BEGIN SSH SIGNATURE-----"}"""

    private class Relay(
        var challenge: RelayAnswer,
        var register: RelayAnswer = RelayAnswer.Answered(buildJsonObject { put("state", JsonPrimitive("trusted")) }),
    ) {
        val posts = mutableListOf<Pair<String, JsonObject>>()
        suspend fun post(path: String, body: JsonObject): RelayAnswer {
            posts += path to body
            return if (path == DeviceKeyRegistration.CHALLENGE) challenge else register
        }
    }

    private fun minted() = RelayAnswer.Answered(buildJsonObject { put("challenge", JsonPrimitive(challenge)) })

    @Test
    fun signInAsksForAChallengeSignsItForThisAccountAndRegisters() = runTest {
        val relay = Relay(minted())
        val signed = mutableListOf<Pair<String, String>>()
        val keys = DeviceKeyRegistration(relay::post, { a, c -> signed += a to c; proof }, { "Pixel 9" })
        assertEquals(DeviceKeyRegistration.Outcome.Registered("trusted"), keys.registerIfNeeded("user_alice"))
        assertEquals(listOf(DeviceKeyRegistration.CHALLENGE, DeviceKeyRegistration.REGISTER), relay.posts.map { it.first })
        assertEquals(listOf("user_alice" to challenge), signed)
        val body = relay.posts[1].second
        assertEquals(setOf("label", "challenge", "keyA", "proofA"), body.keys)
        assertEquals("Pixel 9", body["label"]!!.jsonPrimitive.content)
        assertEquals(challenge, body["challenge"]!!.jsonPrimitive.content)
        assertEquals("ssh-ed25519 AAAAkey", body["keyA"]!!.jsonPrimitive.content)
    }

    @Test
    fun oncePerAccountAndAgainForTheNext() = runTest {
        val relay = Relay(minted())
        val signed = mutableListOf<String>()
        val keys = DeviceKeyRegistration(relay::post, { a, _ -> signed += a; proof }, { "Pixel" })
        keys.registerIfNeeded("user_alice")
        assertNull(keys.registerIfNeeded("user_alice"))
        keys.registerIfNeeded("user_bob")
        assertEquals(listOf("user_alice", "user_bob"), signed)
        assertNull(keys.registerIfNeeded(""))
    }

    @Test
    fun aSignInDuringALaunchRegistrationIsQueuedNotDropped() = runTest {
        val relay = Relay(minted())
        val gate = CompletableDeferred<Unit>()
        var held = true
        val slow: suspend (String, JsonObject) -> RelayAnswer = { path, body ->
            if (held && path == DeviceKeyRegistration.CHALLENGE) {
                held = false
                gate.await()
            }
            relay.post(path, body)
        }
        val signed = mutableListOf<String>()
        val keys = DeviceKeyRegistration(slow, { a, _ -> signed += a; proof }, { "Pixel" })
        val launch = async { keys.registerIfNeeded("user_alice") }
        runCurrent()
        // Signed out and in as Bob while Alice's is in flight.
        assertNull(keys.registerIfNeeded("user_bob"))
        gate.complete(Unit)
        assertEquals(DeviceKeyRegistration.Outcome.Registered("trusted"), launch.await())
        assertEquals(listOf("user_alice", "user_bob"), signed)
        assertNull(keys.registerIfNeeded("user_bob"))
    }

    @Test
    fun aCoreThatWontSignSendsNothingMoreAndIsTriedAgain() = runTest {
        val relay = Relay(minted())
        var answer: String? = """{"error":"challenge"}"""
        val keys = DeviceKeyRegistration(relay::post, { _, _ -> answer }, { "Pixel" })
        assertEquals(DeviceKeyRegistration.Outcome.NoProof, keys.registerIfNeeded("user_alice"))
        assertEquals(listOf(DeviceKeyRegistration.CHALLENGE), relay.posts.map { it.first })
        answer = proof
        assertEquals(DeviceKeyRegistration.Outcome.Registered("trusted"), keys.registerIfNeeded("user_alice"))
    }

    @Test
    fun eachAnswerComesToAStateOrARefusal() = runTest {
        val cases = listOf(
            Relay(RelayAnswer.Refused(404, null)) to DeviceKeyRegistration.Outcome.Refused(404),
            Relay(RelayAnswer.Unreachable("IOException")) to DeviceKeyRegistration.Outcome.Unreachable,
            Relay(RelayAnswer.Answered(JsonObject(emptyMap()))) to DeviceKeyRegistration.Outcome.Refused(200),
            Relay(minted(), RelayAnswer.Refused(403, null)) to DeviceKeyRegistration.Outcome.Refused(403),
            Relay(minted(), RelayAnswer.Answered(buildJsonObject { put("state", JsonPrimitive("pending")) }))
                to DeviceKeyRegistration.Outcome.Registered("pending"),
            Relay(minted(), RelayAnswer.Answered(buildJsonObject { put("state", JsonPrimitive("owned")) }))
                to DeviceKeyRegistration.Outcome.Refused(200),
        )
        for ((relay, expected) in cases) {
            val keys = DeviceKeyRegistration(relay::post, { _, _ -> proof }, { "Pixel" })
            assertEquals(expected, keys.registerIfNeeded("user_alice"))
        }
    }

    @Test
    fun theCoresAnswerReadsAsAProofAndItsRefusalAsNone() {
        assertEquals("ssh-ed25519 AAAAkey" to "-----BEGIN SSH SIGNATURE-----", DeviceKeyRegistration.proofOf(proof))
        for (refusal in listOf("""{"error":"key"}""", """{"keyA":"ssh-rsa AAAA","proofA":"x"}""", "", "[]")) {
            assertNull(refusal, DeviceKeyRegistration.proofOf(refusal))
        }
    }
}
