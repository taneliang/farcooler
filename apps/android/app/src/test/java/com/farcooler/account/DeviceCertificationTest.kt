package com.farcooler.account

import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds

/**
 * This phone vouching for the device it added to its own account in the
 * ceremony (ov-550): what it signs and for whom, what it posts, and each
 * answer. The relay and the core are faked; the signature is proved in Rust
 * (`device_keys_tests.rs`) and accepted by the relay's tests against the same
 * fixture.
 */
class DeviceCertificationTest {
    private val scanned = "ssh-ed25519 AAAAscanned farcooler-Pixel"
    private val signedAnswer = """{"certificate":"-----BEGIN SSH SIGNATURE-----"}"""

    private class Relay(vararg answers: RelayAnswer) {
        private val queue = answers.toMutableList()
        val posts = mutableListOf<Pair<String, JsonObject>>()
        suspend fun post(path: String, body: JsonObject): RelayAnswer {
            posts += path to body
            return if (queue.isEmpty()) RelayAnswer.Unreachable("none") else queue.removeAt(0)
        }
    }

    private fun refused(status: Int, state: String? = null) =
        RelayAnswer.Refused(status, state?.let { buildJsonObject { put("error", JsonPrimitive("state")); put("state", JsonPrimitive(it)) } })

    private val ok = RelayAnswer.Answered(buildJsonObject { put("state", JsonPrimitive("certified")) })

    private fun certification(
        relay: Relay,
        signedIn: () -> String = { "user_alice" },
        signed: MutableList<Pair<String, String>> = mutableListOf(),
        paused: MutableList<Duration> = mutableListOf(),
        answer: String? = signedAnswer,
    ) = DeviceCertification(
        post = relay::post,
        sign = { a, k -> signed += a to k; answer },
        signedIn = signedIn,
        pause = { paused += it },
        log = {},
    )

    @Test
    fun theScannedKeyIsSignedForThisAccountAndPostedWithItsCertificate() = runTest {
        val relay = Relay(ok)
        val signed = mutableListOf<Pair<String, String>>()
        assertEquals(DeviceCertification.Outcome.CERTIFIED, certification(relay, signed = signed).certifyScanned("user_alice", scanned))
        assertEquals(listOf("user_alice" to scanned), signed)
        assertEquals(listOf(DeviceCertification.CERTIFY), relay.posts.map { it.first })
        val body = relay.posts[0].second
        assertEquals(setOf("keyA", "certificate"), body.keys)
        assertEquals(scanned, body["keyA"]!!.jsonPrimitive.content)
        assertEquals("-----BEGIN SSH SIGNATURE-----", body["certificate"]!!.jsonPrimitive.content)
    }

    @Test
    fun aCodeForAnotherAccountOrWithNobodySignedInIsNeverSigned() = runTest {
        for ((here, code) in listOf("user_alice" to "user_bob", "" to "user_alice", "" to "")) {
            val relay = Relay(ok)
            val signed = mutableListOf<Pair<String, String>>()
            val outcome = certification(relay, signedIn = { here }, signed = signed).certifyScanned(code, scanned)
            assertEquals(DeviceCertification.Outcome.NOT_SIGNED, outcome)
            assertTrue(signed.isEmpty() && relay.posts.isEmpty())
        }
    }

    @Test
    fun aNewDeviceNotRegisteredYetIsWaitedForThenCertified() = runTest {
        val relay = Relay(refused(404), refused(404), ok)
        val paused = mutableListOf<Duration>()
        val signed = mutableListOf<Pair<String, String>>()
        val outcome = certification(relay, paused = paused, signed = signed).certifyScanned("user_alice", scanned)
        assertEquals(DeviceCertification.Outcome.CERTIFIED, outcome)
        assertEquals(listOf(2.seconds, 5.seconds), paused)
        assertEquals(3, relay.posts.size)
        assertEquals(1, signed.size)
    }

    @Test
    fun waitingGivesUpAndStopsWhenTheAccountChanges() = runTest {
        val relay = Relay(*Array(10) { refused(404) })
        assertEquals(DeviceCertification.Outcome.NOT_REGISTERED, certification(relay).certifyScanned("user_alice", scanned))
        assertEquals(6, relay.posts.size)

        var who = "user_alice"
        val switched = Relay(*Array(10) { refused(404) })
        val outcome = DeviceCertification(switched::post, { _, _ -> signedAnswer }, { who }, pause = { who = "user_bob" }, log = {})
            .certifyScanned("user_alice", scanned)
        assertEquals(DeviceCertification.Outcome.NOT_SIGNED, outcome)
        assertEquals(1, switched.posts.size)
    }

    @Test
    fun eachAnswerIsToldApart() = runTest {
        val cases = listOf(
            refused(409, "certified") to DeviceCertification.Outcome.ALREADY_CONFIRMED,
            refused(409, "trusted") to DeviceCertification.Outcome.ALREADY_CONFIRMED,
            refused(409, "revoked") to DeviceCertification.Outcome.REFUSED,
            refused(403) to DeviceCertification.Outcome.REFUSED,
            refused(500) to DeviceCertification.Outcome.FAILED,
            RelayAnswer.Unreachable("offline") to DeviceCertification.Outcome.FAILED,
        )
        for ((answer, expected) in cases) {
            assertEquals(expected, certification(Relay(answer)).certifyScanned("user_alice", scanned))
        }
        val relay = Relay(ok)
        assertEquals(
            DeviceCertification.Outcome.NOT_SIGNED,
            certification(relay, answer = """{"error":"key"}""").certifyScanned("user_alice", scanned),
        )
        assertTrue(relay.posts.isEmpty())
    }
}
