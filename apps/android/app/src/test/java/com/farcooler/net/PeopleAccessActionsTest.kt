package com.farcooler.net

import com.farcooler.model.PeopleAccess
import com.farcooler.model.PersonAccess
import com.farcooler.model.StepUpFailure
import java.util.Base64
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The Access menu and Limit to Viewer on this phone (ov-856): each is one signed request for exactly that person or
 * device, with a prompt that says what it is for, and a refusal is a sentence. AgentKit's `PeopleAccessModel`.
 */
class PeopleAccessActionsTest {
    private val members: JsonObject = Json.parseToJsonElement(
        """{"members":[{"handle":"bob","name":"Bob","devices":["phone-b"],"access":"member"}],"claude_account":"","you":"",
        "device_access":[
          {"client_id":"phone-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
          {"client_id":"phone-b","person":"bob","access":"member","cap":"","check":false,"step_up":"","step_up_code":"","step_up_at":0}]}""",
    ).jsonObject

    private class Core : StepUpCalling {
        val calls = mutableListOf<Pair<String, JsonObject>>()
        var refuse: String? = null


        override suspend fun call(method: String, args: JsonObject): JsonObject {
            calls += method to args
            refuse?.let { throw com.farcooler.core.CoreException("no", word = "resource-conflict", what = it) }
            return if (method == "step_up.begin") {
                buildJsonObject {
                    put("token", "t")
                    put("statement", Base64.getEncoder().encodeToString("s".toByteArray()))
                }
            } else {
                buildJsonObject { put("ok", true) }
            }
        }

        override fun refusalWord(error: Throwable): String? = (error as? com.farcooler.core.CoreException)?.what
    }

    private class Signer : StepUpSigning {
        val prompts = mutableListOf<String>()
        override val publicKey: ByteArray = ByteArray(65)
        override fun makeKey() = publicKey
        override suspend fun sign(statement: ByteArray, reason: String): ByteArray {
            prompts += reason
            return byteArrayOf(1)
        }
    }

    private fun controller(core: Core, signer: Signer) = PeopleAccessController(
        read = { PeopleAccess.of(members, null, "phone-o") },
        stepUp = { StepUp(core, signer, "phone-o", "SHA256:host") },
        runner = "studio",
    )

    @Test
    fun accessAndLimitAreEachOneSignedRequestForThatPersonOrDevice() = runTest {
        val core = Core()
        val signer = Signer()
        val people = controller(core, signer)
        people.load()
        val bob = people.access!!.people.first { it.handle == "bob" }
        people.setAccess(bob, PersonAccess.VIEWER)
        val begin = core.calls.first { it.first == "step_up.begin" }.second
        assertEquals("member.set_access", begin["method"]!!.jsonPrimitive.content)
        assertEquals("bob", begin["args"]!!.jsonObject["handle"]!!.jsonPrimitive.content)
        assertEquals("viewer", begin["args"]!!.jsonObject["access"]!!.jsonPrimitive.content)
        assertEquals(listOf("Make Bob a Viewer on studio."), signer.prompts)

        people.limit(bob.devices.single())
        val limit = core.calls.last { it.first == "step_up.begin" }.second
        assertEquals("client.set_cap", limit["method"]!!.jsonPrimitive.content)
        assertEquals("phone-b", limit["args"]!!.jsonObject["clientId"]!!.jsonPrimitive.content)
        assertEquals("viewer", limit["args"]!!.jsonObject["cap"]!!.jsonPrimitive.content)
        assertEquals("Limit phone-b on studio to Viewer.", signer.prompts.last())
        assertNull(people.note)
    }

    @Test
    fun aRefusalIsASentenceAndNeverTheRunnersWord() = runTest {
        val core = Core().also { it.refuse = "admin_is_owners" }
        val people = controller(core, Signer())
        people.load()
        people.setAccess(people.access!!.people.first { it.handle == "bob" }, PersonAccess.ADMIN)
        assertEquals("Only the Owner can make someone an Admin.", people.note)
        assertTrue(people.note!!.none { it == '_' })
    }

    @Test
    fun aClosedPromptSaysNothing() = runTest {
        val signer = object : StepUpSigning {
            override val publicKey: ByteArray = ByteArray(65)
            override fun makeKey() = publicKey
            override suspend fun sign(statement: ByteArray, reason: String): ByteArray = throw StepUpFailure.Cancelled
        }
        val people = PeopleAccessController(
            read = { PeopleAccess.of(members, null, "phone-o") },
            stepUp = { StepUp(Core(), signer, "phone-o", "SHA256:host") },
            runner = "studio",
        )
        people.load()
        people.limit(people.access!!.people.first { it.handle == "bob" }.devices.single())
        assertNull(people.note)
    }
}
