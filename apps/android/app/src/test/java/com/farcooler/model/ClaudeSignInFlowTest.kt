package com.farcooler.model

import com.farcooler.net.SignInReply
import com.farcooler.net.StartedSignIn
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The sign-in sheet's state when replies land after it closed or restarted (ov-584). */
class ClaudeSignInFlowTest {
    private val page = "https://claude.com/cai/oauth/authorize?code=true"
    private val signedIn = ClaudeLogin(ClaudeLoginState.SIGNED_IN, "alex@example.com")

    private class Api : ClaudeSignInApi {
        val starts = mutableListOf<CompletableDeferred<SignInReply<StartedSignIn>>>()
        val finishes = mutableListOf<CompletableDeferred<SignInReply<ClaudeLogin>>>()
        val cancelled = mutableListOf<String>()
        override suspend fun start() = CompletableDeferred<SignInReply<StartedSignIn>>().also { starts += it }.await()
        override suspend fun finish(session: String, code: String) =
            CompletableDeferred<SignInReply<ClaudeLogin>>().also { finishes += it }.await()
        override suspend fun cancel(session: String) { cancelled += session }
    }

    private val logins = mutableListOf<ClaudeLogin>()
    private val api = Api()

    // Unconfined: a launch runs to its first suspension at once, and resumes when completed.
    private val flow = ClaudeSignInFlow(CoroutineScope(Dispatchers.Unconfined), api, "Studio") { logins += it }

    private fun started(session: String) = SignInReply.Ok(StartedSignIn(session, page))

    @Test
    fun `a start that lands after the sheet closed leaves it closed and cancels its session`() = runBlocking {
        flow.start()
        assertEquals(ClaudeSignInStep.Starting, flow.step.value)
        flow.close()
        assertNull(flow.step.value)
        api.starts[0].complete(started("late"))
        assertNull("the sheet stays closed", flow.step.value)
        assertEquals(listOf("late"), api.cancelled)
    }

    @Test
    fun `a finish that lands after the sheet closed shows no Done, and still updates the row`() = runBlocking {
        flow.start()
        api.starts[0].complete(started("s1"))
        flow.opened(page, PageOpen.OPENED)
        flow.edit("abc#def")
        flow.submit(page)
        assertEquals(ClaudeSignInStep.Finishing, flow.step.value)
        flow.close()
        api.finishes[0].complete(SignInReply.Ok(signedIn))
        assertNull("no Done after a dismiss", flow.step.value)
        assertEquals("the runner did sign them in", listOf(signedIn), logins)
    }

    @Test
    fun `a start overtaken by a newer one is dropped and cancelled`() = runBlocking {
        flow.start()
        flow.start()
        api.starts[0].complete(started("old"))
        assertEquals(ClaudeSignInStep.Starting, flow.step.value)
        assertEquals(listOf("old"), api.cancelled)
        api.starts[1].complete(started("new"))
        assertEquals(ClaudeSignInStep.OpenPage(page), flow.step.value)
    }

    @Test
    fun `closing with a live session cancels it, and Done does not`() = runBlocking {
        flow.start()
        api.starts[0].complete(started("s1"))
        flow.close()
        assertEquals(listOf("s1"), api.cancelled)
        flow.start()
        api.starts[1].complete(started("s2"))
        flow.opened(page, PageOpen.OPENED)
        flow.edit("abc#def")
        flow.submit(page)
        api.finishes[0].complete(SignInReply.Ok(signedIn))
        assertEquals(ClaudeSignInStep.Done("alex@example.com"), flow.step.value)
        flow.close()
        assertEquals(listOf("s1"), api.cancelled)
    }

    @Test
    fun `Try again cancels the old session before starting a new one`() = runBlocking {
        flow.start()
        api.starts[0].complete(started("s1"))
        flow.start()
        assertEquals(listOf("s1"), api.cancelled)
        assertEquals(1 + 1, api.starts.size)
    }

    @Test
    fun `an incomplete code clears its error when the text changes`() = runBlocking {
        flow.start()
        api.starts[0].complete(started("s1"))
        flow.opened(page, PageOpen.OPENED)
        flow.edit("abcdef")
        flow.submit(page)
        assertEquals(ClaudeSignInStep.Failed(ClaudeSignIn.Failure.Incomplete, page), flow.step.value)
        assertTrue("no call for an incomplete code", api.finishes.isEmpty())
        flow.edit("abcdef#")
        assertEquals(ClaudeSignInStep.Code(page), flow.step.value)
    }

    @Test
    fun `a paste past the cap keeps the field and says so`() = runBlocking {
        flow.edit("abc#def")
        flow.edit("x".repeat(ClaudeSignIn.MAX_CODE + 1))
        assertEquals("abc#def", flow.code.value)
        assertEquals("That’s longer than a sign-in code.", flow.codeNote.value)
        flow.edit("abc#defg")
        assertNull(flow.codeNote.value)
    }

    @Test
    fun `no browser is its own sentence, not a Claude Code update`() = runBlocking {
        flow.start()
        api.starts[0].complete(started("s1"))
        flow.opened(page, PageOpen.NO_BROWSER)
        assertEquals(ClaudeSignInStep.Failed(ClaudeSignIn.Failure.NoBrowser, page), flow.step.value)
    }
}
