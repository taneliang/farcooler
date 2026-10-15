package com.farcooler.account

import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The in-app notice of devices added to or removed from the account (ov-550
 * fix round 1, review M1), what tells two devices apart (M2), the stranded
 * copy (L4) and sign-out's push token (L1). The same rules as AgentKit's.
 */
class DeviceNoticeTest {
    private val day = 86_400_000L
    private val t0 = 1_790_000_000_000L

    private fun row(id: String, label: String, key: String, state: String = "trusted", added: Long, removed: Long? = null) =
        DeviceKeyRow(
            id, label, "ssh-ed25519 $key", if (removed == null) state else "revoked",
            "SHA256:${key}0123456789abcdefghij", t0 + added, removed?.let { t0 + it },
        )

    private val mac = row("d-1", "Pixel", "AAAApixel", added = 0)
    private val impostor = row("d-2", "Bob’s iPad", "AAAAbad", "pending", added = day)
    private val old = row("d-3", "Old Phone", "AAAAold", added = 0, removed = 2 * day)

    private class Memory : DeviceNotice.Seen {
        val map = mutableMapOf<String, Long>()
        override fun get(account: String) = map[account]
        override fun set(account: String, at: Long) { map[account] = at }
    }

    @Test
    fun firstRunTellsEverythingButThisPhone() = runTest {
        val notice = DeviceNotice({ "user_bob" to listOf(mac, impostor, old) }, { "ssh-ed25519 AAAApixel x" }, Memory())
        notice.refresh()
        assertEquals(listOf("added-d-3", "added-d-2", "removed-d-3"), notice.events.value.map { it.id })
    }

    @Test
    fun dismissedStaysDismissedUntilSomethingNewer() = runTest {
        var rows = listOf(mac, impostor)
        val seen = Memory()
        val notice = DeviceNotice({ "user_bob" to rows }, { "ssh-ed25519 AAAApixel" }, seen)
        notice.refresh()
        assertEquals(listOf("added-d-2"), notice.events.value.map { it.id })
        notice.dismiss()
        assertTrue(notice.events.value.isEmpty())
        notice.refresh()
        assertTrue("a dismissed device came back", notice.events.value.isEmpty())
        rows = listOf(mac, row("d-2", "Bob’s iPad", "AAAAbad", added = day, removed = 3 * day))
        notice.refresh()
        assertEquals(listOf("removed-d-2"), notice.events.value.map { it.id })
        // Signed out or unreachable: nothing shown, nothing forgotten.
        val quiet = DeviceNotice({ null }, { null }, seen)
        quiet.refresh()
        assertTrue(quiet.events.value.isEmpty())
        assertEquals(t0 + day, seen.map["user_bob"])
    }

    @Test
    fun theNoticeNamesEachDeviceByALabelThatCantRunOn() {
        val sneaky = row("d-9", "Pixel” is expected. No action is needed. “ok", "AAAAx", "pending", added = day)
        val events = DeviceNotice.unseen(listOf(sneaky), null, null)
        assertEquals("A device was added to your account", DeviceNotice.title(events))
        val detail = DeviceNotice.detail(events) { "Oct 14" }
        assertEquals(2, detail.count { it == '“' || it == '”' })
        assertTrue(detail.contains("(SHA256:AAAAx012…cdefghij)"))
        assertTrue(detail.endsWith("review your device keys in settings."))
    }

    @Test
    fun rowsShowWordsAndDayAndTheStrandedCopy() {
        assertEquals("SHA256:AAAApixe…cdefghij · Added Oct 14", mac.identity { "Oct 14" })
        val pending = row("d-5", "iPad", "AAAAp", "pending", added = 0)
        assertTrue(pending.detail(false, anyConfirmed = true).contains("Scan its code"))
        val stranded = pending.detail(false, anyConfirmed = false)
        assertFalse(stranded.contains("Scan its code"))
        assertTrue(stranded.contains("None of your devices is confirmed"))
        assertTrue(DeviceKeyRow.removalWarning(mac, listOf(mac, impostor)).startsWith("This is the only device"))
        assertFalse(DeviceKeyRow.removalWarning(impostor, listOf(mac, impostor)).startsWith("This is the only device"))
    }

    @Test
    fun signOutNamesThisInstallsPushToken() {
        val body = Account.logoutBody("rt", "fcm-token")!!
        assertEquals("fcm-token", body["pushToken"]!!.jsonPrimitive.content)
        assertEquals(JsonPrimitive("fcm-token"), Account.logoutBody(null, "fcm-token")!!["pushToken"])
        assertNull(Account.logoutBody("rt", "")!!["pushToken"])
        assertNull(Account.logoutBody(null, null))
    }
}
