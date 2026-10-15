package com.farcooler.account

import java.io.File
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Starting over with no confirmed device (ruling R-75, ov-561), and labels that
 * can't masquerade (ov-560 R1). The same rules as AgentKit's `DeviceTrustTests`.
 */
class DeviceTrustTest {
    private val day = 86_400_000L
    private val t0 = 1_790_000_000_000L

    private fun row(id: String, label: String, key: String, state: String, added: Long, trusted: Long? = null, reset: Boolean = false) =
        DeviceKeyRow(
            id, label, "ssh-ed25519 $key", state, "SHA256:${key}0123456789abcdefghij",
            createdAt = t0 + added, trustedAt = trusted?.let { t0 + it }, trustReset = reset,
        )

    private val mac = row("d-1", "Pixel", "AAAAmac", "revoked", added = 0, trusted = 0)
    private val phone = row("d-2", "Bob’s Pixel", "AAAAphone", "trusted", added = day, trusted = 3 * day, reset = true)

    @Test
    fun aDeviceTrustedAgainIsAnnouncedAfterItsArrival() {
        val events = DeviceNotice.unseen(listOf(mac, phone), t0 + 2 * day, null)
        assertEquals(listOf("trusted-d-2"), events.map { it.id })
        assertEquals(t0 + 3 * day, events[0].at)
        assertEquals("A device is now confirmed on your account", DeviceNotice.title(events))
        val detail = DeviceNotice.detail(events) { "Oct 17" }
        assertTrue(detail, detail.startsWith("Confirmed “Bob’s Pixel” ("))
        assertTrue(detail, detail.contains("on Oct 17, since none of your devices was confirmed."))
    }

    @Test
    fun aFirstDevicesTrustIsItsArrivalAndThisPhonesOwnIsntNews() {
        val first = row("d-3", "Pixel", "AAAAfirst", "trusted", added = 0, trusted = 0)
        assertEquals(listOf("added-d-3"), DeviceNotice.unseen(listOf(first), null, null).map { it.id })
        assertTrue(DeviceNotice.unseen(listOf(phone), null, "ssh-ed25519 AAAAphone x").isEmpty())
        assertTrue(DeviceNotice.unseen(listOf(phone), t0 + 3 * day, null).isEmpty())
    }

    @Test
    fun aRowSaysHowItWasConfirmedAndAStrandedOneSaysToSignInOnTheDeviceToKeep() {
        assertEquals("Confirmed when it signed in, since none of your devices was confirmed", phone.detail(false))
        assertEquals(
            "Confirmed as the first device on your account",
            row("d-3", "Pixel", "AAAAfirst", "trusted", added = 0, trusted = 0).detail(false),
        )
        val stranded = row("d-4", "Tablet", "AAAAtab", "pending", added = 0).detail(true, anyConfirmed = false)
        assertTrue(stranded, stranded.startsWith("This device. Not confirmed. None of your devices is confirmed"))
        assertTrue(stranded, stranded.contains("Do that on the device you want to keep"))
        assertFalse(stranded, stranded.contains("Far Cooler team"))
        val pending = row("d-4", "Tablet", "AAAAtab", "pending", added = 0)
        assertTrue(DeviceKeyRow.removalWarning(phone, listOf(phone, pending)).contains("becomes confirmed in its place"))
    }

    /** Fix round 1, M1: a new device trusted by the reset is one trust event, and no first device. */
    @Test
    fun aNewDeviceTrustedByTheResetIsAnnouncedAsConfirmedNeverAddedOrFirst() {
        val laptop = row("d-5", "Laptop", "AAAAlap", "trusted", added = 2 * day, trusted = 2 * day, reset = true)
        val events = DeviceNotice.unseen(listOf(mac, laptop), t0 + day, null)
        assertEquals(listOf("trusted-d-5"), events.map { it.id })
        assertEquals("A device is now confirmed on your account", DeviceNotice.title(events))
        assertEquals("Confirmed when it signed in, since none of your devices was confirmed", laptop.detail(false))
    }

    /** Fix round 1, Low 4: removing the last confirmed device from this phone keeps this phone. */
    @Test
    fun removingTheLastConfirmedDeviceStartsOverHereAndOnlyThen() {
        val own = "ssh-ed25519 AAAApad x"
        val pending = row("d-4", "Tablet", "AAAApad", "pending", added = 0)
        val other = row("d-6", "Pixel", "AAAApix", "pending", added = 0)
        assertTrue(DeviceKeyRow.startsOverHere(listOf(pending, other), own))
        assertFalse(DeviceKeyRow.startsOverHere(listOf(pending, phone), own))
        assertFalse(DeviceKeyRow.startsOverHere(listOf(other), own))
        assertFalse(DeviceKeyRow.startsOverHere(listOf(pending), null))
    }

    @Test
    fun meTrustedAtIsRead() = runTest {
        val keys = DeviceKeys { _, _ ->
            RelayAnswer.Answered(buildJsonObject {
                put("devices", buildJsonArray {
                    add(buildJsonObject {
                        put("id", JsonPrimitive("d-2")); put("label", JsonPrimitive("Phone"))
                        put("keyA", JsonPrimitive("ssh-ed25519 AAAAphone")); put("state", JsonPrimitive("trusted"))
                        put("createdAt", JsonPrimitive(t0)); put("trustedAt", JsonPrimitive(t0 + day))
                        put("trustReset", JsonPrimitive(true))
                    })
                })
            })
        }
        val rows = keys.list()!!
        assertEquals(t0 + day, rows[0].trustedAt)
        assertTrue(rows[0].wasTrustedAgain)
    }

    @Test
    fun signingInRegistersAgainEvenWhenThisLaunchAlreadyDid() = runTest {
        val posts = mutableListOf<String>()
        val keys = DeviceKeyRegistration(
            { path, _ ->
                posts += path
                RelayAnswer.Answered(
                    if (path == DeviceKeyRegistration.CHALLENGE) buildJsonObject { put("challenge", JsonPrimitive("ab".repeat(32))) }
                    else buildJsonObject { put("state", JsonPrimitive("trusted")) },
                )
            },
            { _, _ -> """{"keyA":"ssh-ed25519 AAAAkey","proofA":"-----BEGIN SSH SIGNATURE-----"}""" },
            { "Pixel 9" },
        )
        keys.registerIfNeeded("user_bob")
        assertEquals(DeviceKeyRegistration.Outcome.Registered("trusted"), keys.registerAgain("user_bob"))
        assertEquals(4, posts.count())
    }

    /** One table for every platform: the relay, the web and AgentKit read it too. */
    @Test
    fun everyLabelInTheSharedTableIsShownWithoutWhatCouldMasquerade() {
        val cases = Json.parseToJsonElement(table().readText()).jsonObject["cases"] as JsonArray
        assertTrue(cases.size >= 10)
        for (case in cases) {
            val c = case.jsonObject
            val row = DeviceKeyRow("d", c.text("label"), "ssh-ed25519 AAAA", "trusted")
            assertEquals(c.text("why"), c.text("shown"), row.shownLabel)
        }
    }

    private fun JsonObject.text(key: String) = this[key]!!.jsonPrimitive.content

    /** `test/fixtures/device-labels.json`, found by walking up from wherever Gradle runs. */
    private fun table(): File {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val file = File(directory, "test/fixtures/device-labels.json")
            if (file.isFile) return file
            directory = directory.parentFile
        }
        throw AssertionError("Could not find test/fixtures/device-labels.json above ${System.getProperty("user.dir")}.")
    }
}
