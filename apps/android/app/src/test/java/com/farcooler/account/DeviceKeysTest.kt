package com.farcooler.account

import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The account's device keys in Settings, with Remove (ov-549 review M1). */
class DeviceKeysTest {
    private val me = Json.parseToJsonElement(
        """{"user":{"id":"user_alice"},"teams":[],"devices":[
          {"id":"d-1","label":"Pixel","keyA":"ssh-ed25519 AAAApixel","state":"trusted"},
          {"id":"d-2","label":"Old","keyA":"ssh-ed25519 AAAAold","state":"revoked"},
          {"id":"d-3","label":"Stranger","keyA":"ssh-ed25519 AAAAx","state":"pending"}]}""",
    ).jsonObject

    @Test
    fun theKeysComeFromMeWithoutTheRemovedOnes() = runTest {
        val posts = mutableListOf<String>()
        val rows = DeviceKeys { path, _ -> posts += path; RelayAnswer.Answered(me) }.list()!!
        assertEquals(listOf("d-1", "d-3"), rows.map { it.id })
        assertEquals(listOf(true, false), rows.map { it.isConfirmed })
        assertEquals(listOf(DeviceKeys.ME), posts)
        assertNull(DeviceKeys { _, _ -> RelayAnswer.Unreachable("offline") }.list())
    }

    @Test
    fun removeRevokesThatKeyByItsId() = runTest {
        val bodies = mutableListOf<Pair<String, JsonObject>>()
        assertTrue(DeviceKeys { path, body -> bodies += path to body; RelayAnswer.Answered(JsonObject(emptyMap())) }.remove("d-3"))
        assertEquals(DeviceKeys.REVOKE, bodies.single().first)
        assertEquals("d-3", bodies.single().second["device"]!!.jsonPrimitive.content)
        assertFalse(DeviceKeys { _, _ -> RelayAnswer.Refused(404, null) }.remove("d-3"))
    }

    @Test
    fun thisDeviceIsTheRowHoldingItsOwnKey() {
        val row = DeviceKeyRow("d-1", "Pixel", "ssh-ed25519 AAAApixel", "trusted")
        assertTrue(row.isHeldBy("ssh-ed25519 AAAApixel farcooler-Pixel"))
        assertFalse(row.isHeldBy("ssh-ed25519 AAAAother"))
        assertFalse(row.isHeldBy(null))
        assertTrue(row.detail(isThisDevice = true).startsWith("This device. "))
        assertTrue(DeviceKeyRow("d-2", "iPad", "ssh-ed25519 AAAAx", "pending").detail(false).contains("Scan its code"))
    }
}
