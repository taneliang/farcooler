package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.model.PeopleAccess
import com.farcooler.model.StepUpFailure
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What the People screen's buttons do (ov-856, slice 4): each is one step-up, saying what it is for, and the screen
 * reads the list again only when the runner took it. AgentKit's `PeopleAccessModel`, over fakes.
 */
class PeopleAccessControllerTest {
    private fun members(thisKey: String): JsonObject = Json.parseToJsonElement(
        """{"members":[{"handle":"cosmo","name":"Cosmo","devices":["phone-c","pad-c"],"access":"member"}],
          "claude_account":"","you":"",
          "device_access":[
            {"client_id":"phone-o","person":"","access":"owner","cap":"","check":false,"step_up":"$thisKey","step_up_code":"","step_up_at":5},
            {"client_id":"ipad-o","person":"","access":"owner","cap":"","check":false,"step_up":"proposed","step_up_code":"418207","step_up_at":9},
            {"client_id":"phone-c","person":"cosmo","access":"member","cap":"","check":false,"step_up":"","step_up_code":"","step_up_at":0},
            {"client_id":"pad-c","person":"cosmo","access":"member","cap":"viewer","check":false,"step_up":"","step_up_code":"","step_up_at":0}]}""",
    ).jsonObject

    private val clients: JsonObject = Json.parseToJsonElement(
        """{"clients":[
          {"clientId":"phone-o","label":"farcooler-iPhone-aaaa1111","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"ipad-o","label":"farcooler-iPad-hhhh8888","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"phone-c","label":"farcooler-Cosmos-iPhone-dddd4444","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"pad-c","label":"farcooler-iPad-eeee5555","shellAccess":false,"foreign":false,"addedBy":"phone-c"}]}""",
    ).jsonObject

    private class Core : StepUpCalling {
        val calls = mutableListOf<Pair<String, JsonObject>>()
        var failFinishWith: Exception? = null
        override suspend fun call(method: String, args: JsonObject): JsonObject {
            calls += method to args
            if (method == "step_up.finish") failFinishWith?.let { throw it }
            return when (method) {
                "step_up.begin" -> buildJsonObject { put("token", "t-1"); put("statement", java.util.Base64.getEncoder().encodeToString(byteArrayOf(1, 2, 3))) }
                "step_up.propose" -> buildJsonObject { put("code", "273905") }
                else -> JsonObject(emptyMap())
            }
        }
        override fun refusalWord(error: Throwable): String? = (error as? CoreException)?.let { it.what ?: it.word }
        val methods get() = calls.map { it.first }
    }

    private class Signer : StepUpSigning {
        val pair = java.security.KeyPairGenerator.getInstance("EC").apply { initialize(java.security.spec.ECGenParameterSpec("secp256r1")) }.generateKeyPair()
        val prompts = mutableListOf<String>()
        var cancels = false
        override val publicKey: ByteArray get() = P256.uncompressed(pair.public)
        override fun makeKey(): ByteArray = publicKey
        override suspend fun sign(statement: ByteArray, reason: String): ByteArray {
            prompts += reason
            if (cancels) throw StepUpFailure.Cancelled
            return byteArrayOf(9)
        }
    }

    private class Fixture(thisKey: String, members: (String) -> JsonObject, clients: JsonObject) {
        val core = Core()
        val signer = Signer()
        var reads = 0
        var key = thisKey
        val controller = PeopleAccessController(
            read = { reads++; PeopleAccess.of(members(key), clients, "phone-o") },
            stepUp = { StepUp(core, signer, "phone-o", "SHA256:pin") },
            runner = "studio",
        )
    }

    private fun fixture(thisKey: String = "trusted") = Fixture(thisKey, ::members, clients)

    @Test
    fun removingADeviceAndWhatItAddedIsOnePromptThatSaysSoAndThenReadsAgain() = runTest {
        val f = fixture()
        f.controller.load()
        val phone = f.controller.access!!.people.first { it.handle == "cosmo" }.devices[0]
        assertEquals(1, phone.added)
        f.controller.remove(phone, withAdded = true)
        assertEquals(listOf("Remove Cosmos-iPhone and the device it added."), f.signer.prompts)
        assertEquals(listOf("step_up.begin", "step_up.finish"), f.core.methods)
        val args = f.core.calls[0].second["args"]!!.jsonObject
        assertEquals("phone-c", args["clientId"]!!.jsonPrimitive.content)
        assertEquals("true", args["withAdded"]!!.jsonPrimitive.content)
        assertEquals("read again after a change the runner took", 2, f.reads)
        assertNull(f.controller.note)
    }

    @Test
    fun closingThePromptRemovesNothingAndSaysNothing() = runTest {
        val f = fixture()
        f.signer.cancels = true
        f.controller.load()
        f.controller.remove(f.controller.access!!.people.first { it.handle == "cosmo" }.devices[1], withAdded = false)
        assertEquals(listOf("step_up.begin"), f.core.methods)
        assertNull("closing it is silence", f.controller.note)
        assertEquals(1, f.reads)
    }

    @Test
    fun aRefusalIsASentenceNotTheRunnersWord() = runTest {
        val f = fixture()
        f.core.failFinishWith = CoreException("no", word = "resource-conflict", what = "challenge_expired")
        f.controller.load()
        f.controller.remove(f.controller.access!!.people.first { it.handle == "cosmo" }.devices[1], withAdded = false)
        assertEquals("That took too long. Try again.", f.controller.note)
        assertEquals("nothing was read again for a change that didn't happen", 1, f.reads)
    }

    @Test
    fun anotherDevicesKeyIsAllowedByItsCodeAfterAPromptThatNamesIt() = runTest {
        val f = fixture()
        f.controller.load()
        val ipad = f.controller.access!!.waiting.single()
        f.controller.approve(ipad)
        assertEquals(listOf("Allow iPad to approve changes to who has access."), f.signer.prompts)
        assertEquals("418207", f.core.calls[0].second["args"]!!.jsonObject["code"]!!.jsonPrimitive.content)
        assertEquals("client.approve_step_up", f.core.calls[0].second["method"]!!.jsonPrimitive.content)
    }

    @Test
    fun allowingAdminActionsShowsTheDigitsTheRunnerGave() = runTest {
        val f = fixture(thisKey = "")
        f.controller.load()
        assertNull(f.controller.shownCode)
        f.controller.allowAdminActions()
        assertEquals("273905", f.controller.shownCode)
        assertEquals(listOf("step_up.propose"), f.core.methods)
        assertTrue("no prompt: a proposal signs nothing", f.signer.prompts.isEmpty())
    }

    @Test
    fun aPhoneThatCannotMakeAStepUpSaysSoInsteadOfDoingNothing() = runTest {
        val core = Core()
        val controller = PeopleAccessController(
            read = { PeopleAccess.of(members("trusted"), clients, "phone-o") }, stepUp = { null }, runner = "studio",
        )
        controller.load()
        controller.remove(controller.access!!.people.first { it.handle == "cosmo" }.devices[0], withAdded = false)
        assertEquals("This phone can’t keep an admin key. Use another device.", controller.note)
        assertTrue(core.calls.isEmpty())
        assertNotNull(controller.access)
        assertFalse(controller.access!!.waiting.isEmpty())
    }

    @Test
    fun aRunnerBeforePersonAccessHasNoAccessToShow() = runTest {
        val controller = PeopleAccessController(
            read = { PeopleAccess.of(Json.parseToJsonElement("""{"members":[],"claude_account":"","you":""}""").jsonObject) },
            stepUp = { null }, runner = "studio",
        )
        controller.load()
        assertNull(controller.access)
    }
}
