package com.farcooler.ceremony

import com.farcooler.account.RelayAnswer
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * This phone answering a browser's offer (ov-770): when the scanned offer
 * names a `reply_key`, the reply it built is sealed by the core and posted
 * once, as `{ceremony, account, sealed}`, and the core's code goes on screen.
 * An offer without one is the QR-only ceremony, with nothing sealed or
 * posted. The core and the relay are fakes here; the seal itself, through the
 * JNI, is `NativeBridgeTest`'s, and opening it is the Rust suite's
 * (`the_sealed_reply_over_the_ffi.rs`).
 */
class SealedReplyTest {
    private val replyKey = "YwbAvkATmjREjY8yCsUr87XR8t4kv8l2bK516AhoEWQ"
    private val manifest = """{"v":2,"ceremony":"c-1","account":"user_bob","runners":[]}"""

    private fun offer(replyKey: String?): String {
        val key = replyKey?.let { ",\"reply_key\":\"$it\"" } ?: ""
        return "{\"v\":2,\"key_a\":\"ssh-ed25519 AAAAbrowser\",\"name\":\"Chrome\",\"account\":\"user_bob\"," +
            "\"channel\":\"stable\",\"ceremony\":\"c-1\",\"node_key\":\"\"$key}"
    }

    private class Core(val offer: String, val manifest: String, val sealAnswer: CeremonyCore.Answer) : CeremonyCalls {
        val sealed = mutableListOf<Triple<String, String, Long>>()
        override fun scan(encoded: String, expectingAccount: String, heldMs: Long) = CeremonyCore.Answer.Payload(offer)
        override fun reply(offerJson: String, runners: List<CeremonyRunner>) = CeremonyCore.Answer.Payload(manifest)
        override fun fingerprint(publicKey: String) = "SHA256:test"
        override fun clientId(publicKey: String) = "farcooler-0123456789ab"
        override fun seal(offerJson: String, manifestJson: String, heldMs: Long): CeremonyCore.Answer {
            sealed += Triple(offerJson, manifestJson, heldMs)
            return sealAnswer
        }
    }

    private val sealedAnswer = CeremonyCore.Answer.Payload("""{"sealed":"c2VhbGVkLXJlcGx5","code":"ABC234"}""")

    private suspend fun confirmed(core: Core): Pair<CeremonyStore, List<Triple<String, String, String>>> {
        val store = CeremonyStore(account = "user_bob", accountEmail = "bob@example.com", deviceName = "Bob’s Phone")
        val posts = mutableListOf<Triple<String, String, String>>()
        store.core = core
        var now = 1_000L
        store.clock = { now }
        store.sealedReply.post = { ceremony, account, sealed ->
            posts += Triple(ceremony, account, sealed)
            SealedReplyPost.Outcome.POSTED
        }
        store.read(core.offer, emptyList(), null)
        now = 4_000L
        store.confirm({ _, _ -> ConfirmingTap.Outcome.CONFIRMED }, Enroller { _, _, _, _, _ -> emptyMap() })
        assertEquals(CeremonyStore.Phase.ShowingManifest, store.phase.value)
        assertEquals("the QR reply stays on screen", manifest, store.code.value)
        return store to posts
    }

    @Test
    fun aBrowsersOfferIsSealedByTheCoreAndPostedOnce() = runTest {
        val core = Core(offer(replyKey), manifest, sealedAnswer)
        val (store, posts) = confirmed(core)
        assertEquals("sealed the scan's offer and the reply it built", listOf(Triple(offer(replyKey), manifest, 3_000L)), core.sealed)
        assertEquals(listOf(Triple("c-1", "user_bob", "c2VhbGVkLXJlcGx5")), posts)
        assertEquals("ABC234", store.sealedReply.code.value)
        assertEquals(SealedReply.State.SENT, store.sealedReply.state.value)
    }

    @Test
    fun anOfferWithoutAReplyKeySealsAndPostsNothing() = runTest {
        val core = Core(offer(null), manifest, sealedAnswer)
        val (store, posts) = confirmed(core)
        assertTrue(core.sealed.isEmpty())
        assertTrue(posts.isEmpty())
        assertNull(store.sealedReply.code.value)
        assertEquals(SealedReply.State.NONE, store.sealedReply.state.value)
    }

    @Test
    fun aSealTheCoreRefusesPostsNothing() = runTest {
        for (refusal in listOf(Refusal.Stale, Refusal.Malformed)) {
            val (store, posts) = confirmed(Core(offer(replyKey), manifest, CeremonyCore.Answer.Refused(refusal)))
            assertTrue("$refusal posted", posts.isEmpty())
            assertNull(store.sealedReply.code.value)
            assertEquals(SealedReply.State.NOT_SENT, store.sealedReply.state.value)
        }
    }

    @Test
    fun aPostTheRelayRefusesSaysItWasntSent() = runTest {
        val core = Core(offer(replyKey), manifest, sealedAnswer)
        val store = CeremonyStore(account = "user_bob", accountEmail = "bob@example.com", deviceName = "Bob’s Phone")
        store.core = core
        store.clock = { 0L }
        store.sealedReply.post = { _, _, _ -> SealedReplyPost.Outcome.REFUSED }
        store.read(core.offer, emptyList(), null)
        store.confirm({ _, _ -> ConfirmingTap.Outcome.CONFIRMED }, Enroller { _, _, _, _, _ -> emptyMap() })
        assertEquals(SealedReply.State.NOT_SENT, store.sealedReply.state.value)
        assertNull("a failed post shows the failure, never the code", store.sealedReply.code.value)
        store.reset()
        assertEquals(SealedReply.State.NONE, store.sealedReply.state.value)
        assertNull(store.sealedReply.code.value)
    }

    @Test
    fun theCodeIsNotShownUntilTheRelayAnswers200() = runTest {
        val core = Core(offer(replyKey), manifest, sealedAnswer)
        val store = CeremonyStore(account = "user_bob", accountEmail = "bob@example.com", deviceName = "Bob’s Phone")
        store.core = core
        store.clock = { 0L }
        val relay = kotlinx.coroutines.CompletableDeferred<SealedReplyPost.Outcome>()
        var duringPost: Pair<SealedReply.State, String?>? = null
        store.sealedReply.post = { _, _, _ ->
            duringPost = store.sealedReply.state.value to store.sealedReply.code.value
            relay.await()
        }
        store.read(core.offer, emptyList(), null)
        val confirming = launch(kotlinx.coroutines.Dispatchers.Unconfined) {
            store.confirm({ _, _ -> ConfirmingTap.Outcome.CONFIRMED }, Enroller { _, _, _, _, _ -> emptyMap() })
        }
        assertEquals("posting, with no code up", SealedReply.State.SENDING to null, duringPost)
        assertNull(store.sealedReply.code.value)
        relay.complete(SealedReplyPost.Outcome.POSTED)
        confirming.join()
        assertEquals(SealedReply.State.SENT, store.sealedReply.state.value)
        assertEquals("ABC234", store.sealedReply.code.value)
    }

    @Test
    fun thePostIsTheRelaysThreeFieldsToTheReplyRoute() = runTest {
        val sent = mutableListOf<Pair<String, JsonObject>>()
        val outcome = SealedReplyPost.send({ path, body ->
            sent += path to body
            RelayAnswer.Answered(buildJsonObject {})
        }, "c-1", "user_bob", "c2VhbGVk")
        assertEquals(SealedReplyPost.Outcome.POSTED, outcome)
        assertEquals(listOf("/v1/ceremony/reply"), sent.map { it.first })
        val body = sent.single().second
        assertEquals(setOf("ceremony", "account", "sealed"), body.keys)
        assertEquals("c-1", body["ceremony"]?.jsonPrimitive?.content)
        assertEquals("user_bob", body["account"]?.jsonPrimitive?.content)
        assertEquals("c2VhbGVk", body["sealed"]?.jsonPrimitive?.content)
    }

    @Test
    fun eachRelayAnswerHasItsOutcome() = runTest {
        val cases = listOf(
            RelayAnswer.Refused(409, null) to SealedReplyPost.Outcome.ALREADY_POSTED,
            RelayAnswer.Refused(403, null) to SealedReplyPost.Outcome.REFUSED,
            RelayAnswer.Refused(500, null) to SealedReplyPost.Outcome.FAILED,
            RelayAnswer.Unreachable("signed out") to SealedReplyPost.Outcome.NOT_SIGNED_IN,
            RelayAnswer.Unreachable("timeout") to SealedReplyPost.Outcome.FAILED,
        )
        for ((answer, expected) in cases) {
            assertEquals(expected, SealedReplyPost.send({ _, _ -> answer }, "c", "a", "s"))
        }
    }
}
