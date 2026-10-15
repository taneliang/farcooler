package com.farcooler.account

import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A device trusted by a reset waits out a hold before team admins can add it
 * (ruling R-76, ov-565). The same rules as AgentKit's `DeviceHoldTests`.
 */
class DeviceHoldTest {
    private val t0 = 1_790_000_000_000L
    private val until = t0 + 72 * 3_600_000L
    private val time: (Long) -> String = { if (it == until) "Oct 18, 2026, 3:00 PM" else "elsewhen" }

    private fun row(state: String, reset: Boolean = false, heldUntil: Long? = until) = DeviceKeyRow(
        "d-2", "Bob’s Pixel", "ssh-ed25519 AAAAphone", state, "SHA256:AAAAphone0123456789abcdefghij",
        createdAt = t0, trustedAt = t0, trustReset = reset, heldUntil = heldUntil,
    )

    private val sentence =
        "Your team admins can’t add it to runners until Oct 18, 2026, 3:00 PM, unless one confirms it with you in person."

    @Test
    fun aHeldRowSaysWhenTeamAdminsCanAddItAndStopsOnceTheHoldEnds() {
        val held = row("trusted", reset = true)
        assertEquals(
            "Confirmed when it signed in, since none of your devices was confirmed. $sentence",
            held.detail(false, now = t0, time = time),
        )
        assertTrue(held.detail(true, now = t0, time = time).startsWith("This device. Confirmed when it signed in"))
        assertFalse(held.detail(false, now = until, time = time).contains("team admins"))
        // A device confirmed by a held one is held too, and says so.
        assertTrue(row("certified").detail(false, now = t0, time = time).endsWith(sentence))
        // An unconfirmed row has nothing to wait out.
        assertFalse(row("pending").detail(false, now = t0, time = time).contains("team admins"))
        assertFalse(row("trusted", heldUntil = null).detail(false, now = t0, time = time).contains("team admins"))
    }

    @Test
    fun theBannersResetLineSaysTheDeviceIsHeldWhileItIs() {
        val event = DeviceEvent(DeviceEvent.Kind.TRUSTED, row("trusted", reset = true), t0)
        val detail = DeviceNotice.detail(listOf(event), now = t0, time = time) { "Oct 15" }
        assertTrue(detail, detail.contains("since none of your devices was confirmed. $sentence If you don’t recognize"))
        assertFalse(DeviceNotice.detail(listOf(event), now = until, time = time) { "Oct 15" }.contains("team admins"))
    }

    @Test
    fun meHeldUntilIsRead() = runTest {
        val keys = DeviceKeys { _, _ ->
            RelayAnswer.Answered(buildJsonObject {
                put("devices", buildJsonArray {
                    add(buildJsonObject {
                        put("id", JsonPrimitive("d-2")); put("label", JsonPrimitive("Phone"))
                        put("keyA", JsonPrimitive("ssh-ed25519 AAAAphone")); put("state", JsonPrimitive("trusted"))
                        put("heldUntil", JsonPrimitive(until))
                    })
                    add(buildJsonObject {
                        put("id", JsonPrimitive("d-3")); put("label", JsonPrimitive("Mac"))
                        put("keyA", JsonPrimitive("ssh-ed25519 AAAAmac")); put("state", JsonPrimitive("trusted"))
                        put("heldUntil", JsonNull)
                    })
                })
            })
        }
        val rows = keys.list()!!
        assertEquals(until, rows[0].heldUntil)
        assertNull(rows[1].heldUntil)
    }
}

/** A team admin lifting a hold is news on the banner, read from `me` (ov-565 fix round 1, M1). */
class HoldConfirmationNoticeTest {
    private val t0 = 1_790_000_000_000L
    private val row = DeviceKeyRow(
        "d-2", "Bob’s Pixel", "ssh-ed25519 AAAAphone", "trusted", "SHA256:AAAAphone0123456789abcdefghij",
        createdAt = t0, trustedAt = t0, trustReset = true,
        holdConfirmations = listOf(DeviceKeyRow.HoldConfirmation("Acme‮", t0 + 3_600_000L)),
    )

    @Test
    fun aLiftedHoldIsAnnouncedWithTheTeamsNameEvenForThisPhone() {
        val events = DeviceNotice.unseen(listOf(row), t0, "ssh-ed25519 AAAAphone x")
        assertEquals(listOf(DeviceEvent.Kind.CONFIRMED), events.map { it.kind })
        assertEquals("A team admin confirmed one of your devices", DeviceNotice.title(events))
        val detail = DeviceNotice.detail(events) { "Oct 15" }
        assertTrue(detail, detail.startsWith("An admin of “Acme” confirmed “Bob’s Pixel” ("))
        assertTrue(detail, detail.contains("so that team’s runners can add it now."))
        assertTrue(DeviceNotice.unseen(listOf(row), t0 + 3_600_000L, null).isEmpty())
    }

    @Test
    fun meHoldConfirmationsAreRead() = runTest {
        val keys = DeviceKeys { _, _ ->
            RelayAnswer.Answered(buildJsonObject {
                put("devices", buildJsonArray {
                    add(buildJsonObject {
                        put("id", JsonPrimitive("d-2")); put("label", JsonPrimitive("Phone"))
                        put("keyA", JsonPrimitive("ssh-ed25519 AAAAphone")); put("state", JsonPrimitive("trusted"))
                        put("holdConfirmations", buildJsonArray {
                            add(buildJsonObject { put("team", JsonPrimitive("Acme")); put("at", JsonPrimitive(t0)) })
                        })
                    })
                })
            })
        }
        assertEquals(listOf(DeviceKeyRow.HoldConfirmation("Acme", t0)), keys.list()!![0].holdConfirmations)
    }
}
