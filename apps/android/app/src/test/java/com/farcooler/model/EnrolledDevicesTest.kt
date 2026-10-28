package com.farcooler.model

import com.farcooler.net.revokeArgs
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Who added each device, and what revoking one offers (ov-855). `AgentKitTests/EnrolledDevicesTests.swift` is the
 * Apple half; neither can see the other, which is why both exist.
 */
class EnrolledDevicesTest {
    private val answer = """
        {"clients":[
          {"clientId":"mac-1","label":"Studio Mac","scope":"host_admin","foreign":false,"shellAccess":false,"addedBy":""},
          {"clientId":"mac-1","label":"Studio Mac","scope":"host_admin","foreign":false,"shellAccess":true,"addedBy":""},
          {"clientId":"phone-1","label":"Alexandra’s iPhone","scope":"control","foreign":false,"addedBy":"mac-1"},
          {"clientId":"web-1","label":"Alexandra’s Browser","scope":"read","foreign":false,"addedBy":"phone-1"},
          {"clientId":"web-2","label":"","scope":"read","foreign":false,"addedBy":"phone-1"},
          {"clientId":"old-1","label":"Old Tablet","scope":"read","foreign":false,"addedBy":"gone-9"},
          {"clientId":"","label":"by hand","scope":"unspecified","foreign":true,"addedBy":""}
        ]}
    """.trimIndent()

    private fun list() = requireNotNull(EnrolledDevices.of(Json.parseToJsonElement(answer).jsonObject))
    private fun device(id: String) = list().devices.first { it.clientId == id }

    @Test
    fun `a Mac's two lines are one row and a hand-written line has none`() {
        assertEquals(listOf("mac-1", "phone-1", "web-1", "web-2", "old-1"), list().devices.map { it.clientId })
    }

    @Test
    fun `a row names the device that added it and none for one nobody added`() {
        val l = list()
        assertEquals("Added by Studio Mac", l.addedByLine(device("phone-1")))
        assertEquals("Added by Alexandra’s iPhone", l.addedByLine(device("web-1")))
        assertNull(l.addedByLine(device("mac-1")))
    }

    @Test
    fun `an adder that is no longer enrolled is said so`() {
        assertEquals("Added by a device that was removed", list().addedByLine(device("old-1")))
    }

    @Test
    fun `a device with no name is titled by its id`() {
        assertEquals("web-2", device("web-2").title)
    }

    @Test
    fun `what a device added includes what those added in turn and never itself`() {
        val l = list()
        assertEquals(setOf("phone-1", "web-1", "web-2"), l.addedBy(device("mac-1")).map { it.clientId }.toSet())
        assertFalse(l.addedBy(device("phone-1")).any { it.clientId == "phone-1" })
    }

    @Test
    fun `a loop in who-added-whom ends`() {
        val a = EnrolledDevices.Device("a", "A", addedBy = "b")
        val b = EnrolledDevices.Device("b", "B", addedBy = "a")
        assertEquals(listOf("b"), EnrolledDevices(listOf(a, b)).addedBy(a).map { it.clientId })
    }

    @Test
    fun `a device that added others offers both choices, counted`() {
        val choices = list().revokeChoices(device("phone-1"))
        assertEquals("Revoke this", EnrolledDevices.JUST_THIS_TITLE)
        assertEquals("Revoke this and the 2 it added", choices.withAddedTitle)
        assertTrue(choices.message, choices.message.contains("Alexandra’s Browser, web-2"))
    }

    @Test
    fun `a device that added none offers only the one choice`() {
        assertNull(list().revokeChoices(device("web-1")).withAddedTitle)
    }

    @Test
    fun `anything but a client list is not one`() {
        assertNull(EnrolledDevices.of(Json.parseToJsonElement("{}").jsonObject))
    }

    @Test
    fun `only the second choice sends withAdded`() {
        assertEquals(setOf("clientId"), revokeArgs("phone-1", false).keys)
        assertEquals(setOf("clientId", "withAdded"), revokeArgs("phone-1", true).keys)
        assertEquals("true", revokeArgs("phone-1", true)["withAdded"].toString())
    }

    @Test
    fun `the daily limit has its own sentence, only for add_limit`() {
        assertTrue(RunnerRefusal.ADD_LIMIT_SENTENCE.contains("five"))
        val limited = troubleFor("resource-conflict", "raw", "generic", "add_limit")
        assertEquals(RunnerRefusal.ADD_LIMIT_SENTENCE, limited.sentence)
        assertNull(limited.transcript)
        val taken = troubleFor("resource-conflict", "raw", "generic", "device_taken")
        assertEquals(RunnerRefusal.RESOURCE_CONFLICT.sentence, taken.sentence)
        val other = troubleFor("not-found", "raw", "generic", "add_limit")
        assertEquals(RunnerRefusal.NOT_FOUND.sentence, other.sentence)
    }
}
