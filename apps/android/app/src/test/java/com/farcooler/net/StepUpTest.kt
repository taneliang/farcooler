package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.model.AppLock
import com.farcooler.model.PeopleAccess
import com.farcooler.model.PersonAccess
import com.farcooler.model.StepUpFailure
import com.farcooler.model.StepUpReason
import com.farcooler.model.StepUpWords
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.Signature
import java.security.spec.ECGenParameterSpec
import java.util.Base64
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * Step-up from the app's side (ov-856, slice 4): the order of the calls, one prompt per action, the keyless
 * transition, and the words. A software P-256 key stands where the Keystore would, signing the same way (DER over
 * SHA-256 of the statement). AgentKit's `StepUpTests`, in Kotlin.
 */
class StepUpTest {
    private class Core : StepUpCalling {
        val calls = mutableListOf<Pair<String, JsonObject>>()
        var refuseFirstPlainWith: String? = null
        val statement = "farcooler-step-up-v1 statement".toByteArray()

        class Refused(val word: String) : Exception(word)

        override suspend fun call(method: String, args: JsonObject): JsonObject {
            calls += method to args
            return when (method) {
                "step_up.begin" -> buildJsonObject {
                    put("token", "t-1")
                    put("statement", Base64.getEncoder().encodeToString(statement))
                }
                "step_up.finish" -> buildJsonObject { put("clients", "signed") }
                "step_up.propose" -> buildJsonObject { put("clientId", "phone-o"); put("state", "proposed"); put("code", "418207") }
                else -> {
                    refuseFirstPlainWith?.let {
                        refuseFirstPlainWith = null
                        throw Refused(it)
                    }
                    buildJsonObject { put("clients", "plain") }
                }
            }
        }

        override fun refusalWord(error: Throwable): String? = (error as? Refused)?.word
        val methods get() = calls.map { it.first }
    }

    private class Signer(hasKey: Boolean = true) : StepUpSigning {
        var pair: KeyPair? = if (hasKey) newPair() else null
        val prompts = mutableListOf<String>()
        var cancels = false
        var made = 0

        override val publicKey: ByteArray? get() = pair?.let { P256.uncompressed(it.public) }

        override fun makeKey(): ByteArray {
            made++
            pair = newPair()
            return publicKey!!
        }

        override suspend fun sign(statement: ByteArray, reason: String): ByteArray {
            prompts += reason
            if (cancels) throw StepUpFailure.Cancelled
            return Signature.getInstance("SHA256withECDSA").run {
                initSign(pair!!.private)
                update(statement)
                sign()
            }
        }

        companion object {
            fun newPair(): KeyPair = KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.generateKeyPair()
        }
    }

    private fun step(core: Core, signer: Signer) = StepUp(core, signer, "phone-o", "SHA256:host")
    private fun args(vararg pairs: Pair<String, String>) = buildJsonObject { pairs.forEach { (k, v) -> put(k, v) } }

    @Test
    fun aDeviceWithATrustedKeySignsBeginOnePromptFinishWithThatSignature() = runTest {
        val core = Core()
        val signer = Signer()
        val out = step(core, signer).perform("client.revoke", args("clientId" to "pad-x"), "Remove iPad from studio.", hasTrustedKey = true)
        assertEquals("signed", out["clients"]!!.jsonPrimitive.content)
        assertEquals("the plain call is never made", listOf("step_up.begin", "step_up.finish"), core.methods)
        val begin = core.calls[0].second
        assertEquals("client.revoke", begin["method"]!!.jsonPrimitive.content)
        assertEquals("phone-o", begin["clientId"]!!.jsonPrimitive.content)
        assertEquals("SHA256:host", begin["hostPin"]!!.jsonPrimitive.content)
        assertEquals("pad-x", begin["args"]!!.jsonObject["clientId"]!!.jsonPrimitive.content)
        assertEquals("one prompt, saying what it's for", listOf("Remove iPad from studio."), signer.prompts)
        val finish = core.calls[1].second
        assertEquals("t-1", finish["token"]!!.jsonPrimitive.content)
        val signature = Base64.getDecoder().decode(finish["signature"]!!.jsonPrimitive.content)
        val verify = Signature.getInstance("SHA256withECDSA").apply { initVerify(signer.pair!!.public); update(core.statement) }
        assertTrue("the DER signature over the statement the core built", verify.verify(signature))
    }

    @Test
    fun withoutATrustedKeyTheOldWayStandsAndNobodyIsAskedForAFace() = runTest {
        val core = Core()
        val signer = Signer()
        val out = step(core, signer).perform("client.revoke", args("clientId" to "x"), "r", hasTrustedKey = false)
        assertEquals("plain", out["clients"]!!.jsonPrimitive.content)
        assertEquals(listOf("client.revoke"), core.methods)
        assertTrue(signer.prompts.isEmpty())
    }

    @Test
    fun aRunnerThatAsksForASignatureAnywayGetsOneOnceAndOnlyForThatWord() = runTest {
        val core = Core().apply { refuseFirstPlainWith = "step_up_needed" }
        step(core, Signer()).perform("client.revoke", args("clientId" to "x"), "r", hasTrustedKey = false)
        assertEquals(listOf("client.revoke", "step_up.begin", "step_up.finish"), core.methods)

        val other = Core().apply { refuseFirstPlainWith = "scope-denied" }
        try {
            step(other, Signer()).perform("client.revoke", args("clientId" to "x"), "r", hasTrustedKey = false)
            fail("served")
        } catch (e: Core.Refused) {
            assertEquals("any other refusal is the caller's to show", listOf("client.revoke"), other.methods)
        }

        val keyless = Core().apply { refuseFirstPlainWith = "step_up_needed" }
        try {
            step(keyless, Signer(hasKey = false)).perform("client.revoke", args(), "r", hasTrustedKey = false)
            fail("served")
        } catch (e: Core.Refused) {
            assertEquals(listOf("client.revoke"), keyless.methods)
        }
    }

    @Test
    fun closingThePromptSendsNothing() = runTest {
        val core = Core()
        val signer = Signer().apply { cancels = true }
        try {
            step(core, signer).perform("client.revoke", args(), "r", hasTrustedKey = true)
            fail("served")
        } catch (e: StepUpFailure.Cancelled) {
            assertEquals("the action is never finished", listOf("step_up.begin"), core.methods)
        }
    }

    @Test
    fun registeringMakesTheKeyOnceAndShowsTheDigitsTheRunnerGave() = runTest {
        val core = Core()
        val fresh = Signer(hasKey = false)
        assertEquals("418207", step(core, fresh).register())
        assertEquals(1, fresh.made)
        val sent = Base64.getDecoder().decode(core.calls.first().second["publicKey"]!!.jsonPrimitive.content)
        assertArrayEquals("the key sent is the key held", fresh.publicKey, sent)
        assertEquals(65, sent.size)
        assertEquals(4, sent[0].toInt())

        val held = Signer()
        step(Core(), held).register()
        assertEquals("an existing key is offered, not replaced", 0, held.made)
    }

    @Test
    fun aPromptSaysWhatItIsForAsTheDesignWordsIt() {
        assertEquals("Remove Cosmo from studio.", StepUpReason.remove("Cosmo", "studio"))
        assertEquals("Add Cosmo’s iPhone to studio as a Member.", StepUpReason.add("Cosmo’s iPhone", "studio", PersonAccess.MEMBER))
        assertEquals("Add x to studio as an Admin.", StepUpReason.add("x", "studio", PersonAccess.ADMIN))
        assertEquals("Remove E-Liang’s iPad and the 2 devices it added.", StepUpReason.removeDevice("E-Liang’s iPad", 2, "studio"))
        assertEquals("Remove iPad and the device it added.", StepUpReason.removeDevice("iPad", 1, "studio"))
        assertEquals("Remove iPad from studio.", StepUpReason.removeDevice("iPad", 0, "studio"))
    }

    @Test
    fun aRefusalIsASentenceAndClosingThePromptIsSilence() {
        val say = { word: String -> StepUpWords.sentence(StepUpFailure.Refused(word), "studio", "phone") }
        assertEquals("Allow admin actions on this phone from your Mac first.", say("step_up_needed"))
        assertEquals("Allow admin actions on this phone from your Mac first.", say("key_untrusted"))
        assertEquals("Update Far Cooler on studio to change access here.", say("step_up"))
        assertEquals("That took too long. Try again.", say("challenge_expired"))
        assertEquals("Couldn’t do that. Try again.", say("unheard_of"))
        assertNull(StepUpWords.sentence(StepUpFailure.Cancelled, "studio", "phone"))
        assertFalse("no runner word reaches the screen", say("host_pin_mismatch")!!.contains("host_pin"))
    }

    @Test
    fun theKeystoresPointIsTheSixtyFiveBytesTheRunnerTakes() {
        val pair = Signer.newPair()
        val point = P256.uncompressed(pair.public)
        assertEquals(65, point.size)
        assertEquals(4, point[0].toInt())
        val w = (pair.public as java.security.interfaces.ECPublicKey).w
        assertEquals(w.affineX, java.math.BigInteger(1, point.copyOfRange(1, 33)))
        assertEquals(w.affineY, java.math.BigInteger(1, point.copyOfRange(33, 65)))
    }

    @Test
    fun theCoreExceptionsWhatIsTheWordAnAppReads() {
        val calls = CoreStepUpCalls { _, _ -> JsonObject(emptyMap()) }
        assertEquals("step_up_needed", calls.refusalWord(CoreException("no", word = "resource-conflict", what = "step_up_needed")))
        assertEquals("resource-conflict", calls.refusalWord(CoreException("no", word = "resource-conflict")))
        assertNull(calls.refusalWord(IllegalStateException("x")))
    }

    // MARK: - The app lock

    @Test
    fun awayForFiveMinutesLocksAndFourMinutesFiftyNineDoesnt() {
        val near = AppLock().apply { didLeave(0); didReturn(AppLock.IDLE_LIMIT_MS - 1) }
        assertFalse(near.isLocked)
        val far = AppLock().apply { didLeave(0); didReturn(AppLock.IDLE_LIMIT_MS) }
        assertTrue(far.isLocked)
        assertEquals(300_000L, AppLock.IDLE_LIMIT_MS)
    }

    @Test
    fun aLaunchIsLockedAndAnUnlockLastsUntilTheNextLongAbsence() {
        val lock = AppLock.atLaunch()
        assertTrue(lock.isLocked)
        lock.unlock()
        lock.didLeave(0); lock.didReturn(60_000)
        assertFalse(lock.isLocked)
        lock.didLeave(0); lock.didReturn(600_000)
        assertTrue(lock.isLocked)
    }

    @Test
    fun itCoversTerminalsAndAdminScreensAndNothingElse() {
        val lock = AppLock(isOn = true, locked = true)
        assertTrue(lock.covers(AppLock.Screen.TERMINAL) && lock.covers(AppLock.Screen.ADMIN) && !lock.covers(AppLock.Screen.OTHER))
        assertFalse("turned off", AppLock(isOn = false, locked = true).covers(AppLock.Screen.TERMINAL))
        assertFalse("unlocked", AppLock().covers(AppLock.Screen.TERMINAL))
    }

    @Test
    fun aResumeWithNoRecordOfLeavingIsLockedNeverOpen() {
        val lock = AppLock().apply { didReturn(0) }
        assertTrue(lock.isLocked)
        assertNotNull(PeopleAccess.REMOVE_DEVICE)
    }
}
