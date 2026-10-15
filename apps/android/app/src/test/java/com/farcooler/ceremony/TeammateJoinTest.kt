package com.farcooler.ceremony

import com.farcooler.data.Reach
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * This device asks before joining a teammate's runners (ov-543), as the phone
 * does (ov-525): a reply that says another account granted it is held until the
 * person says Join, and Cancel adds nothing. A reply from your own account is
 * done without asking.
 */
class TeammateJoinTest {
    private fun runner(label: String) = CeremonyRunner(
        id = "alice@$label", label = label, alias = label, user = "alice", hostKey = "",
        reach = Reach.Direct("$label.example.com", 22), pending = false,
    )

    private fun store() = CeremonyStore(account = "user_bob", accountEmail = "bob@example.com", deviceName = "Bob’s Pixel")

    private fun reply(grantedBy: String?, vararg labels: String) =
        CeremonyManifest(runners = labels.map(::runner), grantedBy = grantedBy)

    @Test
    fun `a teammate's reply is held until the person says join`() {
        val store = store()
        store.receive(reply("alice@example.com", "build-box"))
        assertEquals(CeremonyStore.Phase.ConfirmingJoin, store.phase.value)
        assertEquals("alice@example.com", store.grantedBy.value)
        store.join()
        assertEquals(CeremonyStore.Phase.Done, store.phase.value)
        assertEquals(listOf("build-box"), store.granted.value.map { it.label })
    }

    @Test
    fun `cancelling adds nothing`() {
        val store = store()
        store.receive(reply("alice@example.com", "build-box"))
        store.cancelJoin()
        assertTrue("Cancel left runners to adopt", store.granted.value.isEmpty())
        assertNull(store.grantedBy.value)
        // A late Join has nothing to say yes to.
        store.join()
        assertTrue(store.phase.value != CeremonyStore.Phase.Done)
        assertTrue(store.granted.value.isEmpty())
    }

    @Test
    fun `your own devices' reply is done without asking`() {
        val store = store()
        store.receive(reply(null, "build-box"))
        assertEquals(CeremonyStore.Phase.Done, store.phase.value)
        // A blank name is no other account.
        val blank = store()
        blank.receive(reply("  ", "build-box"))
        assertEquals(CeremonyStore.Phase.Done, blank.phase.value)
    }

    @Test
    fun `the words are the phone's and say what the code says`() {
        assertEquals("Join your teammate’s runner “build-box”?", TeammateJoinCopy.title(listOf(runner("build-box"))))
        assertEquals("Join your teammate’s runners?", TeammateJoinCopy.title(listOf(runner("a"), runner("b"))))
        assertTrue(TeammateJoinCopy.detail("alice@example.com").startsWith("This code says alice@example.com added"))
    }

    @Test
    fun `a long label is cut in the middle`() {
        val long = "Platform Builds, EU West (Shared) and Staging Runner 7"
        val cut = TeammateJoinCopy.shortened(long)
        assertEquals(TeammateJoinCopy.LABEL_LIMIT, cut.length)
        assertTrue(cut.startsWith("Platform") && cut.endsWith("Runner 7") && "…" in cut)
        assertEquals("build-box", TeammateJoinCopy.shortened("build-box"))
        assertTrue(TeammateJoinCopy.title(listOf(runner(long))).contains(cut))
    }
}
