package com.farcooler.account

import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * "Make a new key" (ov-569), with fake key slots, a scripted relay and fake
 * runners: the same cases as AgentKit's `PhoneRekeyTests`.
 */
class PhoneRekeyTest {
    private val old = "ssh-ed25519 AAAAold farcooler-Pixel"
    private val new = "ssh-ed25519 AAAAnew farcooler-Pixel"

    private inner class Slots : PhoneKeySlots {
        override var currentPublicKey: String? = old
        override var nextPublicKey: String? = null
        override var previousPublicKey: String? = null
        override var runnersOnPrevious: Set<String> = emptySet()
        var refusesToMake = false
        val proved = mutableListOf<String>()
        val certified = mutableListOf<String>()

        override fun makeNext(): String? {
            if (refusesToMake) return null
            nextPublicKey = new
            return new
        }
        /** The marks as they stood when the new key was put in use. */
        var marksAtPromote: Set<String>? = null
        override fun promote(): Boolean {
            marksAtPromote = runnersOnPrevious
            val next = nextPublicKey ?: return false
            previousPublicKey = currentPublicKey
            currentPublicKey = next
            nextPublicKey = null
            return true
        }
        override fun forgetPrevious() { previousPublicKey = null }
        override fun proveNext(account: String, challenge: String): String? {
            proved += account
            return nextPublicKey?.let { """{"keyA":"$it","proofA":"proof-by-next"}""" }
        }
        override fun certifyByCurrent(account: String, keyA: String): String? {
            certified += keyA
            return """{"certificate":"certificate-by-current"}"""
        }
    }

    /** A relay holding the account's device rows, answering each route. */
    private inner class Relay(rows: List<JsonObject>) {
        var rows = rows.toMutableList()
        var registerStatus = 200
        val posts = mutableListOf<Pair<String, JsonObject>>()
        fun body(suffix: String) = posts.lastOrNull { it.first.endsWith(suffix) }?.second
        val paths get() = posts.map { it.first.removePrefix("/v1/directory/") }

        suspend fun post(path: String, body: JsonObject): RelayAnswer {
            posts += path to body
            return when (path) {
                DeviceKeyRegistration.CHALLENGE -> ok("challenge" to "a".repeat(64))
                DeviceKeyRegistration.REGISTER -> if (registerStatus != 200) RelayAnswer.Refused(registerStatus, null) else {
                    rows += row("d-new", new, "pending")
                    ok("device" to "d-new", "state" to "pending")
                }
                DeviceKeys.ME -> RelayAnswer.Answered(buildJsonObject { put("devices", JsonArray(rows)) })
                DeviceCertification.CERTIFY -> {
                    rows = rows.map {
                        if (it["id"]?.jsonPrimitive?.content != "d-new") it
                        else JsonObject(it + mapOf("state" to JsonPrimitive("certified"), "certifiedBy" to JsonPrimitive("d-old")))
                    }.toMutableList()
                    ok("state" to "certified")
                }
                DeviceKeys.REVOKE -> ok("ok" to "true")
                else -> RelayAnswer.Unreachable("none")
            }
        }
    }

    private fun ok(vararg fields: Pair<String, String>) =
        RelayAnswer.Answered(buildJsonObject { for ((k, v) in fields) put(k, JsonPrimitive(v)) })

    private fun row(id: String, keyA: String, state: String, label: String = "Pixel") = buildJsonObject {
        put("id", JsonPrimitive(id)); put("label", JsonPrimitive(label))
        put("keyA", JsonPrimitive(keyA)); put("state", JsonPrimitive(state))
    }

    private val runners = listOf(RekeyRunner("r1", "studio"), RekeyRunner("r2", "build-box"))

    private fun rekey(
        slots: Slots,
        relay: Relay?,
        moves: Map<String, RunnerMove> = emptyMap(),
        moved: MutableList<String> = mutableListOf(),
    ) = PhoneRekey(
        keys = slots,
        post = relay?.let { it::post } ?: { _, _ -> RelayAnswer.Unreachable("signed out") },
        signedIn = { if (relay == null) "" else "user_bob" },
        label = { "Pixel" },
        runners = { runners },
        move = { moved += it.id; moves[it.id] ?: RunnerMove.MOVED },
    )

    @Test
    fun aRevokedPhoneMakesANewKeyRegistersItMovesItsRunnersAndWaitsToBeScanned() = runTest {
        val slots = Slots()
        val relay = Relay(listOf(row("d-mac", "ssh-ed25519 AAAAmac", "trusted"), row("d-old", old, "revoked")))
        val moved = mutableListOf<String>()
        val rekey = rekey(slots, relay, moved = moved)
        rekey.run()

        assertTrue(rekey.status.value.finished)
        assertEquals(new, slots.currentPublicKey)
        assertNull("the old key was kept with every runner moved", slots.previousPublicKey)
        assertEquals(listOf("user_bob"), slots.proved)
        assertEquals(new, relay.body("devices/register")?.get("keyA")?.jsonPrimitive?.content)
        assertEquals("pending", rekey.status.value.registered)
        assertTrue("a removed key signed a certificate", slots.certified.isEmpty())
        assertTrue(rekey.status.value.needsScan)
        assertEquals(listOf("r1", "r2"), moved)
        assertFalse(relay.paths.contains("devices/revoke"))
        assertTrue(rekey.status.value.oldRemoved)
    }

    @Test
    fun aKeyStillConfirmedConfirmsItsReplacementAndHandsItItsPlace() = runTest {
        val slots = Slots()
        val relay = Relay(listOf(row("d-old", old, "trusted"), row("d-mac", "ssh-ed25519 AAAAmac", "certified")))
        val rekey = rekey(slots, relay)
        rekey.run()

        assertTrue(rekey.status.value.finished)
        assertEquals(listOf(new), slots.certified)
        assertFalse(rekey.status.value.needsScan)
        assertEquals("d-old", relay.body("devices/revoke")?.get("device")?.jsonPrimitive?.content)
        assertEquals("d-new", relay.body("devices/revoke")?.get("successor")?.jsonPrimitive?.content)
        assertTrue(relay.paths.indexOf("devices/certify") < relay.paths.indexOf("devices/revoke"))
    }

    @Test
    fun theOldKeysRowIsFoundByTheKeyNotByALabel() = runTest {
        val slots = Slots()
        val relay = Relay(listOf(row("d-fake", "ssh-ed25519 AAAAfake", "trusted"), row("d-old", old, "pending")))
        val rekey = rekey(slots, relay)
        rekey.run()
        assertTrue("a certificate was signed on another device's row", slots.certified.isEmpty())
        assertTrue(rekey.status.value.needsScan)
        assertEquals("d-old", relay.body("devices/revoke")?.get("device")?.jsonPrimitive?.content)
        assertNull(relay.body("devices/revoke")?.get("successor"))
    }

    @Test
    fun aRunnerNotReachedKeepsTheOldKeyUntilThePhoneReachesIt() = runTest {
        val slots = Slots()
        val relay = Relay(listOf(row("d-old", old, "trusted")))
        val rekey = rekey(slots, relay, moves = mapOf("r2" to RunnerMove.NOT_REACHED))
        rekey.run()

        assertEquals(setOf("r2"), slots.runnersOnPrevious)
        assertEquals("the old key was deleted while a runner needs it", old, slots.previousPublicKey)
        assertEquals(listOf("r2"), rekey.runnersLeft.map { it.id })
        assertEquals(PhoneKeyChoice.PREVIOUS, PhoneKeyChoice.forRunner("r2", slots.runnersOnPrevious, true))
        assertEquals(PhoneKeyChoice.CURRENT, PhoneKeyChoice.forRunner("r1", slots.runnersOnPrevious, true))

        rekey.move = { RunnerMove.MOVED }
        rekey.moveIfNeeded("r2")
        assertTrue(slots.runnersOnPrevious.isEmpty())
        assertNull(slots.previousPublicKey)
    }

    @Test
    fun aKeyStoreThatWontMakeAKeyChangesNothing() = runTest {
        val slots = Slots().apply { refusesToMake = true }
        val relay = Relay(emptyList())
        val moved = mutableListOf<String>()
        val rekey = rekey(slots, relay, moved = moved)
        rekey.run()
        assertEquals(PhoneRekeyFailure.KEY_STORE, rekey.status.value.failure)
        assertEquals(old, slots.currentPublicKey)
        assertTrue(relay.posts.isEmpty())
        assertTrue(moved.isEmpty())
    }

    @Test
    fun aRefusedRegistrationLeavesTheOldKeyInUseAndARetryReusesTheNewKey() = runTest {
        val slots = Slots()
        val relay = Relay(listOf(row("d-old", old, "trusted"))).apply { registerStatus = 500 }
        val moved = mutableListOf<String>()
        val rekey = rekey(slots, relay, moved = moved)
        rekey.run()
        assertEquals(PhoneRekeyFailure.REGISTER, rekey.status.value.failure)
        assertEquals(old, slots.currentPublicKey)
        assertEquals(new, slots.nextPublicKey)
        assertTrue("runners moved to a key the account doesn't have", moved.isEmpty())

        relay.registerStatus = 200
        slots.refusesToMake = true
        rekey.run()
        assertTrue(rekey.status.value.finished)
        assertEquals(new, slots.currentPublicKey)
    }

    @Test
    fun signedOutTheRunnersStillMoveAndTheRelayIsNotAsked() = runTest {
        val slots = Slots()
        val moved = mutableListOf<String>()
        val rekey = rekey(slots, null, moved = moved)
        rekey.run()
        assertTrue(rekey.status.value.finished)
        assertEquals(new, slots.currentPublicKey)
        assertEquals(listOf("r1", "r2"), moved)
        assertTrue(slots.proved.isEmpty())
        assertEquals("Sign in to add the new key to your account.", PhoneRekey.accountLine(false, rekey.status.value))
    }

    @Test
    fun theOtherKeyIsTriedOnceAndOnlyWhileThePhoneHasBoth() {
        assertEquals(PhoneKeyChoice.PREVIOUS, PhoneKeyChoice.CURRENT.alternate(true))
        assertEquals(PhoneKeyChoice.CURRENT, PhoneKeyChoice.PREVIOUS.alternate(true))
        assertNull(PhoneKeyChoice.CURRENT.alternate(false))
        assertEquals(PhoneKeyChoice.CURRENT, PhoneKeyChoice.forRunner("r1", setOf("r1"), false))
    }

    @Test
    fun everyRunnerIsMarkedBeforeTheNewKeyIsPutInUse() = runTest {
        val slots = Slots()
        rekey(slots, null, moves = mapOf("r1" to RunnerMove.NOT_REACHED, "r2" to RunnerMove.NOT_REACHED)).run()
        assertEquals("the new key was in use before the runners were marked", setOf("r1", "r2"), slots.marksAtPromote)
        assertEquals(old, slots.previousPublicKey)
    }
}
