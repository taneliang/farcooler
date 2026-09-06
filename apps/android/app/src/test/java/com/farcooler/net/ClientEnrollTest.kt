package com.farcooler.net

import com.farcooler.ceremony.CeremonyCore
import com.farcooler.ceremony.CeremonyOffer
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * What one `client.enroll` carries, and how the node key gets into it.
 *
 * **The rule.** A device's tailcat node public key reaches a runner's
 * `authorized_keys` line by exactly one route: the daemon writes it into the
 * forced command when a `client.enroll` names one. Nothing else in the tree
 * writes a node key onto another device's line, and a runner's tunnel admits
 * nobody whose key is not on one.
 *
 * **The failure it guards.** A pairing that dropped this field wrote a perfectly
 * good line and the phone that had just been paired to that runner could not get
 * in: `no_answer` after 10.1 s, against 0.5 s to `connected` once the key was on
 * the line by hand. Nothing fails at the time — the enrollment answers yes, the
 * ceremony completes, the runner is listed — so the field going missing again
 * would look exactly like everything working.
 */
class ClientEnrollTest {
    /** A real one: 43 characters of unpadded base64-URL, `_` included. */
    private val nodeKey = "3klO7naorDKjqf2sm4MV0zWlyTdpZn4Blq03K_crbwc"

    @Test
    fun `the enrollment carries the new device's node key`() {
        val args = Connection.enrollArgs(
            publicKey = "ssh-ed25519 AAAAC3Nz",
            label = "iPhone 17",
            clientId = "farcooler-1",
            nodeKey = nodeKey,
        )

        assertEquals(JsonPrimitive(nodeKey), args["nodeKey"])
    }

    /**
     * A device with no key sends the field empty rather than not at all. Both
     * mean "this device asked for no tunnel" to the client core; sending it says
     * more, and a caller reading this payload beside `ClientEnroll` can see
     * every field it can carry.
     */
    @Test
    fun `a device with no node key still enrolls`() {
        val args = Connection.enrollArgs(
            publicKey = "ssh-ed25519 AAAAC3Nz",
            label = "iPhone 17",
            clientId = "farcooler-1",
            nodeKey = "",
        )

        assertEquals(JsonPrimitive(""), args["nodeKey"])
    }

    /**
     * `control`, always, and no way to ask for a shell.
     *
     * `read` is a narrowing done afterwards in Settings › Devices, and a plain
     * line is a shell — which is every power the account has. `shellAccess`
     * being absent is what the core reads as the restricted line, and the
     * absence of any way to ask for more from a phone is the guard rail. It sits
     * in this test because the node key arrives next to it: a payload builder
     * that gained one field is a payload builder somebody may add a second to.
     */
    @Test
    fun `a phone can only ask for the restricted line at control`() {
        val args = Connection.enrollArgs(
            publicKey = "ssh-ed25519 AAAAC3Nz",
            label = "iPhone 17",
            clientId = "farcooler-1",
            nodeKey = nodeKey,
        )

        assertEquals(JsonPrimitive("control"), args["scope"])
        assertNull("a phone asked for a shell: $args", args["shellAccess"])
    }

    /**
     * And the field the key is read OUT of, which is the other half of the
     * route: `node_key` on the wire, [CeremonyOffer.nodeKey] here. A `SerialName`
     * that did not match would leave every offer carrying an empty key, which is
     * a device that reads as having none — and pairs, silently, without a
     * tunnel.
     */
    @Test
    fun `an offer's node key is read off the wire`() {
        val scanned = """
            {"v":2,"key_a":"ssh-ed25519 AAAAC3Nz","name":"iPhone",
             "account":"user_01","channel":"local","ceremony":"49bf22e1",
             "node_key":"$nodeKey"}
        """.trimIndent()

        val offer = CeremonyCore.decode(scanned, CeremonyOffer.serializer())

        assertEquals(nodeKey, offer?.nodeKey)
    }

    /**
     * A code with no such field at all is a v=1 device, and it decodes to an
     * empty key rather than failing the whole ceremony. Refusing here would
     * refuse a device an enrollment it is entitled to, over a field that only
     * decides whether a tunnel is offered.
     */
    @Test
    fun `an offer with no node key decodes to none`() {
        val scanned = """
            {"v":1,"key_a":"ssh-ed25519 AAAAC3Nz","name":"iPhone",
             "account":"user_01","channel":"local","ceremony":"49bf22e1"}
        """.trimIndent()

        val offer = CeremonyCore.decode(scanned, CeremonyOffer.serializer())

        assertEquals("", offer?.nodeKey)
    }
}
