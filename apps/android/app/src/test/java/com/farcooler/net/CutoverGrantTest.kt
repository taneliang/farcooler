package com.farcooler.net

import com.farcooler.data.Reach
import com.farcooler.data.Runner
import com.farcooler.model.CutoverGrant
import com.farcooler.model.GrantNotSent
import com.farcooler.model.StepUpFailure
import java.security.KeyPairGenerator
import java.security.Signature
import java.security.spec.ECGenParameterSpec
import java.util.Base64
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * The owner's "Allow cutover once" from this phone (ov-974, ov-977): signed with the step-up key after the screen
 * lock, and never sent plain, which would grant nothing. The runner's half (a signed answer is stamped `device`, an
 * unsigned or replayed one isn't) is `step_up_phone_tests` in the client core.
 */
class CutoverGrantTest {
    private val runner = Runner(id = "r1", label = "studio", reach = Reach.Direct("192.168.1.10", 22), user = "me")

    private fun connection() = Connection(
        runner,
        object : com.farcooler.data.ReviewStorage {
            override fun read(key: String): String? = null
            override fun write(key: String, value: String) = Unit
            override fun remove(key: String) = Unit
        },
        MutableStateFlow(""),
        CoroutineScope(StandardTestDispatcher()),
    )

    private class Core : StepUpCalling {
        val calls = mutableListOf<Pair<String, JsonObject>>()

        override suspend fun call(method: String, args: JsonObject): JsonObject {
            calls += method to args
            return when (method) {
                "step_up.begin" -> buildJsonObject {
                    put("token", "t-1")
                    put("statement", Base64.getEncoder().encodeToString("statement".toByteArray()))
                }
                else -> buildJsonObject { put("body", CutoverGrant.ALLOW) }
            }
        }

        override fun refusalWord(error: Throwable): String? = null
    }

    private class Signer(hasKey: Boolean = true, val cancels: Boolean = false) : StepUpSigning {
        private val pair = if (hasKey) KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.generateKeyPair() else null
        val prompts = mutableListOf<String>()

        override val publicKey: ByteArray? get() = pair?.let { P256.uncompressed(it.public) }

        override fun makeKey(): ByteArray = throw StepUpFailure.KeyUnavailable

        override suspend fun sign(statement: ByteArray, reason: String): ByteArray {
            prompts += reason
            if (cancels) throw StepUpFailure.Cancelled
            return Signature.getInstance("SHA256withECDSA").run { initSign(pair!!.private); update(statement); sign() }
        }
    }

    @Test
    fun onlyTheOptionsOwnWordsAreTheGrant() {
        assertTrue(CutoverGrant.isGrant(" allow cutover ONCE \n"))
        assertFalse(CutoverGrant.isGrant("Allow cutover once, after lunch"))
        assertFalse(CutoverGrant.isGrant("Wait for me"))
    }

    @Test
    fun theGrantIsSignedAfterOnePromptAndNeverSentPlain() = runTest {
        val core = Core()
        val signer = Signer()
        connection().answerDecision("task-1", "Allow cutover once") { StepUp(core, signer, "phone-o", "SHA256:host") }
        assertEquals("begin and finish, and no plain task.note", listOf("step_up.begin", "step_up.finish"), core.calls.map { it.first })
        val begin = core.calls[0].second
        assertEquals("task.note", begin["method"]!!.jsonPrimitive.content)
        val args = begin["args"]!!.jsonObject
        assertEquals(listOf("task-1", "answer", CutoverGrant.ALLOW), listOf("task", "kind", "body").map { args[it]!!.jsonPrimitive.content })
        assertEquals(listOf("Allow your orchestrator on studio to run the cutover once."), signer.prompts)
    }

    @Test
    fun aPhoneWithNoKeyIsToldHowAndSendsNothing() = runTest {
        for (flow in listOf<StepUp?>(null, StepUp(Core(), Signer(hasKey = false), "phone-o", "SHA256:host"))) {
            try {
                connection().answerDecision("task-1", "Allow cutover once") { flow }
                fail("sent")
            } catch (e: GrantNotSent) {
                assertEquals("Allow admin actions on this phone from your Mac first.", e.sentence)
            }
        }
        try {
            connection().answerDecision("task-1", "Allow cutover once")
            fail("sent with no way to sign")
        } catch (e: GrantNotSent) {
            assertTrue(e.sentence.isNotEmpty())
        }
    }

    @Test
    fun closingThePromptSendsNothingAndSaysNothing() = runTest {
        val core = Core()
        try {
            connection().answerDecision("task-1", "Allow cutover once") { StepUp(core, Signer(cancels = true), "phone-o", "SHA256:host") }
            fail("sent")
        } catch (e: GrantNotSent) {
            assertEquals("", e.sentence)
        }
        assertEquals(listOf("step_up.begin"), core.calls.map { it.first })
    }
}
