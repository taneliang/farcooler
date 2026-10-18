package com.farcooler.model

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** A member signing in to Claude from the phone (ov-584): who is offered it, the page it opens, and the sentences. */
class ClaudeSignInTest {
    private fun build(vararg caps: Capability) = DaemonBuild("1", true, "linux", caps.map { it.wire }.toSet())
    private val offering = build(Capability.TERMINALS, Capability.CLAUDE_SIGN_IN)
    private fun json(text: String) = Json.parseToJsonElement(text) as JsonObject

    @Test
    fun `the section is a member's own device on a runner that offers it`() {
        assertTrue(ClaudeSignIn.offered(offering, "alex"))
        // The owner's phone is the empty handle: no section.
        assertFalse(ClaudeSignIn.offered(offering, ""))
        assertFalse(ClaudeSignIn.offered(offering, "(released)"))
        // Not known yet: nothing drawn, then drawn.
        assertFalse(ClaudeSignIn.offered(offering, null))
        // A runner without the capability, or before capabilities at all.
        assertFalse(ClaudeSignIn.offered(build(Capability.TERMINALS), "alex"))
        assertFalse(ClaudeSignIn.offered(DaemonBuild("1", true, "linux"), "alex"))
        assertFalse(ClaudeSignIn.offered(null, "alex"))
    }

    @Test
    fun `only the two Claude pages over https open`() {
        val good = "https://claude.com/cai/oauth/authorize?code=true&client_id=abc&state=xyz"
        assertTrue(ClaudeSignIn.isSignInPage(good))
        assertTrue(ClaudeSignIn.isSignInPage("https://platform.claude.com/oauth/authorize?client_id=abc"))
        for (bad in listOf(
            "http://claude.com/cai/oauth/authorize",
            "https://claude.com/oauth/authorize",
            "https://platform.claude.com/cai/oauth/authorize",
            "https://claude.ai/cai/oauth/authorize",
            "https://evil.com/cai/oauth/authorize",
            "https://claude.com.evil.com/cai/oauth/authorize",
            "https://evilclaude.com/cai/oauth/authorize",
            "https://claude.com@evil.com/cai/oauth/authorize",
            "https://user:pw@claude.com/cai/oauth/authorize",
            "https://claude.com:8443/cai/oauth/authorize",
            "https://claude.com/cai/oauth/authorize/../../x",
            "https://claude.com/cai/oauth/authorize/extra",
            "https://claude.com\\@evil.com/cai/oauth/authorize",
            "https://claude.com/cai/oauth/authorize?x=a b",
            "https://claude.com/cai/oauth/authorize?x=a\nb",
            "javascript:alert(1)",
            "intent://claude.com/cai/oauth/authorize#Intent;scheme=https;end",
            "",
        )) {
            assertFalse("should refuse $bad", ClaudeSignIn.isSignInPage(bad))
        }
    }

    @Test
    fun `a page past the length cap is refused`() {
        val base = "https://claude.com/cai/oauth/authorize?s="
        assertTrue(ClaudeSignIn.isSignInPage(base + "a".repeat(ClaudeSignIn.MAX_URL - base.length)))
        assertFalse(ClaudeSignIn.isSignInPage(base + "a".repeat(ClaudeSignIn.MAX_URL - base.length + 1)))
    }

    @Test
    fun `a code needs content and a hash`() {
        assertNull(ClaudeSignIn.code("   \n"))
        assertEquals("abc#def", ClaudeSignIn.code("  abc#def \n"))
        assertTrue(ClaudeSignIn.isWholeCode("abc#def"))
        assertFalse(ClaudeSignIn.isWholeCode("abcdef"))
    }

    private fun refusal(word: String?, what: String? = null) = CoreException("runner said no", word, what)

    @Test
    fun `what the runner refused with picks the sentence`() {
        val r = "Studio"
        assertEquals(ClaudeSignIn.Failure.Incomplete, ClaudeSignIn.failure(refusal("invalid-argument", "sign_in_code"), r))
        assertEquals(ClaudeSignIn.Failure.Refused, ClaudeSignIn.failure(refusal("conflict", "code_refused"), r))
        assertEquals(ClaudeSignIn.Failure.TooOld(r), ClaudeSignIn.failure(refusal("conflict", "sign_in_too_old"), r))
        assertEquals(ClaudeSignIn.Failure.TooOld(r), ClaudeSignIn.failure(refusal("conflict", "sign_in_unreadable"), r))
        assertEquals(ClaudeSignIn.Failure.TooOften, ClaudeSignIn.failure(refusal("resource-conflict", "sign_in_too_often"), r))
        assertEquals("Too many sign-in attempts. Wait a minute, then try again.", ClaudeSignIn.Failure.TooOften.sentence)
        // A missing session is an expired link on the finish call only.
        assertEquals(ClaudeSignIn.Failure.Expired, ClaudeSignIn.failure(refusal("not-found"), r, finishing = true))
        assertEquals(ClaudeSignIn.Failure.Generic, ClaudeSignIn.failure(refusal("not-found"), r, finishing = false))
        // Refusals that aren't known ones are not "isn't answering".
        assertEquals(ClaudeSignIn.Failure.Generic, ClaudeSignIn.failure(refusal("conflict", "own_device_only"), r))
        assertEquals(ClaudeSignIn.Failure.Generic, ClaudeSignIn.failure(refusal("permission-denied", "not_yours"), r))
        assertEquals(ClaudeSignIn.Failure.Generic, ClaudeSignIn.failure(refusal(null), r))
        // The link failing is.
        assertEquals(ClaudeSignIn.Failure.Silent(r), ClaudeSignIn.failure(DisconnectedException("gone"), r))
        assertEquals(ClaudeSignIn.Failure.Silent(r), ClaudeSignIn.failure(refusal(RunnerRefusal.TIMED_OUT_WORD), r))
        assertEquals(ClaudeSignIn.Failure.Silent(r), ClaudeSignIn.failure(java.io.IOException("x"), r))
        assertEquals("That code didn’t work. Open the sign-in page again to get a new one.", ClaudeSignIn.Failure.Refused.sentence)
        assertEquals("Try again", ClaudeSignIn.Failure.Refused.action)
        assertEquals("Start again", ClaudeSignIn.Failure.Expired.action)
        assertEquals("Couldn’t sign in. Try again.", ClaudeSignIn.Failure.Generic.sentence)
        assertEquals(ClaudeSignIn.NO_APP, ClaudeSignIn.Failure.NoBrowser.sentence)
        assertEquals("Studio isn’t answering. Check your connection and try again.", ClaudeSignIn.Failure.Silent(r).sentence)
        assertEquals("Couldn’t start sign-in on Studio. Claude Code there needs to be updated.", ClaudeSignIn.Failure.TooOld(r).sentence)
    }

    @Test
    fun `a failure that says try again offers Try again, and the ones that can't be retried don't`() {
        val r = "Studio"
        assertEquals("Try again", ClaudeSignIn.Failure.Generic.action)
        assertEquals("Try again", ClaudeSignIn.Failure.Silent(r).action)
        assertEquals("Try again", ClaudeSignIn.Failure.Refused.action)
        assertNull(ClaudeSignIn.Failure.TooOften.action)
        assertNull(ClaudeSignIn.Failure.NoBrowser.action)
        assertNull(ClaudeSignIn.Failure.TooOld(r).action)
        assertNull(ClaudeSignIn.Failure.Incomplete.action)
    }

    @Test
    fun `my login is the member list row for who I am`() {
        val reply = json(
            """{"you":"alex","members":[
              {"handle":"sam","claude_login":{"state":"signed_in","email":"sam@example.com"}},
              {"handle":"alex","claude_login":{"state":"stopped_working","email":"alex@example.com"}}]}""",
        )
        assertEquals(ClaudeLogin(ClaudeLoginState.STOPPED_WORKING, "alex@example.com"), ClaudeSignIn.loginIn(reply, "alex"))
        assertEquals(ClaudeLogin(ClaudeLoginState.SIGNED_IN, "sam@example.com"), ClaudeSignIn.loginIn(reply, "sam"))
        assertNull(ClaudeSignIn.loginIn(reply, "nobody"))
        // A runner before the field: not signed in, not unknown.
        assertEquals(ClaudeLogin.NONE, ClaudeSignIn.loginIn(json("""{"members":[{"handle":"alex"}]}"""), "alex"))
    }

    @Test
    fun `the words are the Mac and iPhone's`() {
        assertEquals("Signed in as", ClaudeSignIn.SIGNED_IN_AS)
        assertEquals("Getting a sign-in link from Studio…", ClaudeSignIn.starting("Studio"))
        assertEquals("Sign out of Claude on Studio?", ClaudeSignIn.signOutTitle("Studio"))
        assertTrue(ClaudeSignIn.trust("Studio").startsWith("Your Claude login is kept on Studio. Its owner, and anyone with Control of it, could use it."))
    }
}
