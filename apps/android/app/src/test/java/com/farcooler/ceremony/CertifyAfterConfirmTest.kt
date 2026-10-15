package com.farcooler.ceremony

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * This phone vouches for the device it adds to its own account only once the
 * person has confirmed and the reply is built (ov-550 review L5): never on the
 * scan, never on a cancelled or refused tap, and never when the reply can't be
 * made. The core and the clock are answered by fakes.
 */
class CertifyAfterConfirmTest {
    private val keyA = "ssh-ed25519 AAAAphone farcooler-Pixel"
    private val offer =
        """{"v":2,"key_a":"$keyA","name":"Bob’s Pixel","account":"user_bob","channel":"stable","ceremony":"c-1","node_key":""}"""

    private class Core(var reply: CeremonyCore.Answer) : CeremonyCalls {
        override fun scan(encoded: String, expectingAccount: String, heldMs: Long) = CeremonyCore.Answer.Payload(
            """{"v":2,"key_a":"ssh-ed25519 AAAAphone farcooler-Pixel","name":"Bob’s Pixel","account":"$expectingAccount","channel":"stable","ceremony":"c-1","node_key":""}""",
        )
        override fun reply(offerJson: String, runners: List<CeremonyRunner>) = reply
        override fun fingerprint(publicKey: String) = "SHA256:test"
        override fun clientId(publicKey: String) = "farcooler-0123456789ab"
    }

    private fun scanned(reply: CeremonyCore.Answer = CeremonyCore.Answer.Payload("""{"runners":[]}""")):
        Pair<CeremonyStore, MutableList<Pair<String, String>>> {
        val store = CeremonyStore(account = "user_bob", accountEmail = "bob@example.com", deviceName = "Bob’s Phone")
        val certified = mutableListOf<Pair<String, String>>()
        store.certify = { account, key -> certified += account to key }
        store.core = Core(reply)
        store.clock = { 1_000L }
        store.read(offer, emptyList(), null)
        assertEquals(CeremonyStore.Phase.Confirming, store.phase.value)
        assertTrue("vouched for at the scan", certified.isEmpty())
        return store to certified
    }

    private val enroller = Enroller { _, _, _, _, _ -> emptyMap() }

    @Test
    fun confirmingCertifiesTheScannedKeyOnceTheReplyIsBuilt() = runTest {
        val (store, certified) = scanned()
        store.confirm({ _, _ -> ConfirmingTap.Outcome.CONFIRMED }, enroller)
        assertEquals(CeremonyStore.Phase.ShowingManifest, store.phase.value)
        assertEquals(listOf("user_bob" to keyA), certified)
    }

    @Test
    fun aCancelledOrRefusedTapCertifiesNothing() = runTest {
        for (outcome in listOf(ConfirmingTap.Outcome.CANCELLED, ConfirmingTap.Outcome.REFUSED)) {
            val (store, certified) = scanned()
            store.confirm({ _, _ -> outcome }, enroller)
            assertTrue("$outcome vouched", certified.isEmpty())
        }
    }

    @Test
    fun aReplyThatCantBeBuiltCertifiesNothing() = runTest {
        val (store, certified) = scanned(CeremonyCore.Answer.Refused(Refusal.Unknown))
        store.confirm({ _, _ -> ConfirmingTap.Outcome.CONFIRMED }, enroller)
        assertTrue(store.phase.value is CeremonyStore.Phase.Refused)
        assertTrue(certified.isEmpty())
    }
}
