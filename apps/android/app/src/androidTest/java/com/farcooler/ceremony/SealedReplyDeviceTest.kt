package com.farcooler.ceremony

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.farcooler.data.Reach
import com.farcooler.data.Runner
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * This phone answering a browser's offer through the real core (ov-770): the
 * store reads the offer, confirms, and the core seals; the relay is a list.
 * Then the test does what the browser does with the post, through the same
 * JNI, and opens it with the browser's key.
 *
 * On a device or emulator only, like `NativeBridgeTest`: there is no
 * `libfarcooler_jni.so` on a desktop JVM.
 */
@RunWith(AndroidJUnit4::class)
class SealedReplyDeviceTest {
    /** The relay fixture's throwaway key pair (`services/relay/test/fixtures/ceremony-reply.json`). */
    private val replySecret = "WJUKRfiOBgbYeCZumaFOinssQJqCkAFD5Y1HKRKAAFw"
    private val replyKey = "YwbAvkATmjREjY8yCsUr87XR8t4kv8l2bK516AhoEWQ"
    private val browserKey =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFJhEUXBcOjzsos5aFyw6hfDrERWHei9akIudYJGG2gR browser"
    private val hostKey = "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4"

    private fun browserOffer(): String {
        val made = CeremonyCore.offer("Chrome", "user_bob", browserKey, null) as CeremonyCore.Answer.Payload
        val fields = kotlinx.serialization.json.Json.parseToJsonElement(made.json).jsonObject
        return buildJsonObject {
            fields.forEach { (k, v) -> put(k, v) }
            put("reply_key", JsonPrimitive(replyKey))
        }.toString()
    }

    @Test
    fun theSealedReplyThePhonePostsOpensWithTheBrowsersKey() = runBlocking {
        val offer = browserOffer()
        val store = CeremonyStore(account = "user_bob", accountEmail = "bob@example.com", deviceName = "Bob’s Phone")
        val posts = mutableListOf<Triple<String, String, String>>()
        store.sealedReply.post = { ceremony, account, sealed ->
            posts += Triple(ceremony, account, sealed)
            SealedReplyPost.Outcome.POSTED
        }
        val cosmo = Runner(label = "Cosmo", reach = Reach.Direct("cosmo.local", 22), user = "bob", fingerprint = hostKey)
        store.read(offer, listOf(cosmo), cosmo)
        store.confirm({ _, _ -> ConfirmingTap.Outcome.CONFIRMED }, Enroller { _, _, _, _, _ -> emptyMap() })
        assertEquals(CeremonyStore.Phase.ShowingManifest, store.phase.value)

        val (ceremony, account, sealed) = posts.single()
        assertEquals("user_bob", account)
        assertTrue(offer.contains("\"ceremony\":\"$ceremony\""))
        assertFalse(sealed.contains(hostKey))

        val opened = CeremonyCore.takeSealed(sealed, replySecret, offer) as CeremonyCore.Answer.Payload
        assertTrue(opened.json.contains(hostKey))
        val body = Json.parseToJsonElement(opened.json).jsonObject
        assertEquals(Json.parseToJsonElement(store.code.value), body["manifest"])
        assertEquals("the opener's code is the phone's", body["code"]?.jsonPrimitive?.content, store.sealedReply.code.value)
        assertEquals(6, store.sealedReply.code.value?.length)
    }
}
