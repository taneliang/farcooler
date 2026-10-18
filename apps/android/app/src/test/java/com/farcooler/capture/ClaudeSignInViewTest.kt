package com.farcooler.capture

import android.view.View
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.Modifier
import androidx.compose.ui.node.RootForTest
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ActivityScenario
import com.farcooler.model.ClaudeLogin
import com.farcooler.model.ClaudeLoginState
import com.farcooler.model.ClaudeSignIn
import com.farcooler.model.ClaudeSignInStep
import com.farcooler.model.PageOpen
import com.farcooler.ui.ClaudeSection
import com.farcooler.ui.ClaudeSignInSheet
import com.farcooler.ui.FarCoolerTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * The Claude section and its sign-in sheet (ov-584): the app's own composables,
 * their words, their buttons pressed through semantics, and the pictures with a
 * long email and a long runner name. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class ClaudeSignInViewTest {
    private val email = "alexandra.montgomery-fitzgerald@platform-infrastructure.example-corporation.com"
    private val longRunner = "Platform Builds, EU West (Shared) and Staging Runner 7"
    private val page = "https://claude.com/cai/oauth/authorize?code=true"
    private val signedIn = ClaudeLogin(ClaudeLoginState.SIGNED_IN, "alex@example.com")

    private fun roots(view: View): List<RootForTest> = when {
        view is RootForTest -> listOf(view)
        view is ViewGroup -> (0 until view.childCount).flatMap { roots(view.getChildAt(it)) }
        else -> emptyList()
    }

    private fun texts(node: SemanticsNode, out: MutableList<String>) {
        node.config.getOrNull(SemanticsProperties.Text)?.forEach { out.add(it.text) }
        node.children.forEach { texts(it, out) }
    }

    private fun tagged(node: SemanticsNode, tag: String): SemanticsNode? =
        if (node.config.getOrNull(SemanticsProperties.TestTag) == tag) node
        else node.children.firstNotNullOfOrNull { tagged(it, tag) }

    private fun press(node: SemanticsNode, tag: String) {
        tagged(node, tag)!!.config[SemanticsActions.OnClick].action!!.invoke()
    }

    private fun shown(content: @androidx.compose.runtime.Composable () -> Unit, then: (SemanticsNode, List<String>) -> Unit) {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme { content() } } }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val root = roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode
                then(root, mutableListOf<String>().also { texts(root, it) })
            }
            ShadowLooper.idleMainLooper()
        }
    }

    @Test
    fun `not signed in, the button asks and nothing is signed out`() {
        var signIn = 0
        var signOut = 0
        shown({ ClaudeSection(ClaudeLogin.NONE, "Studio", onSignIn = { signIn += 1 }, onSignOut = { signOut += 1 }) }) { root, said ->
            assertTrue(said.toString(), ClaudeSignIn.BODY in said)
            assertTrue(said.toString(), "Sign in to Claude…" in said)
            assertTrue(said.toString(), said.any { it.startsWith("Your Claude login is kept on Studio.") })
            assertEquals(null, tagged(root, "claude-sign-out"))
            assertEquals(0, signIn)
            press(root, "claude-sign-in")
        }
        assertEquals(1, signIn)
        assertEquals(0, signOut)
    }

    @Test
    fun `signed in, the row names the account and both buttons act`() {
        var signIn = 0
        var signOut = 0
        shown({ ClaudeSection(signedIn, "Studio", onSignIn = { signIn += 1 }, onSignOut = { signOut += 1 }) }) { root, said ->
            assertTrue(said.toString(), "alex@example.com" in said)
            assertFalse(said.toString(), said.any { it.startsWith("Claude · ") })
            assertTrue(said.toString(), "Sign in again…" in said)
            assertFalse(said.toString(), ClaudeSignIn.SIGN_IN in said)
            press(root, "claude-sign-out")
            press(root, "claude-sign-in-again")
        }
        assertEquals(1, signIn)
        assertEquals(1, signOut)
    }

    @Test
    fun `nothing is drawn before the runner has said`() {
        shown({ ClaudeSection(null, "Studio", onSignIn = {}, onSignOut = {}) }) { root, _ ->
            assertEquals(null, tagged(root, "claude-section"))
        }
    }

    @Test
    fun `Sign In stays off until the field holds a code, then asks for the whole code`() {
        var submitted = 0
        // Empty: the button does not act.
        shown({ ClaudeSignInSheet(ClaudeSignInStep.Code(page), "Studio", "  ", null, {}, {}, { submitted += 1 }, {}, {}) }) { root, said ->
            assertTrue(said.toString(), ClaudeSignIn.CODE_BODY in said)
            val finish = tagged(root, "claude-finish")!!
            assertTrue("disabled", finish.config.contains(SemanticsProperties.Disabled))
        }
        shown({ ClaudeSignInSheet(ClaudeSignInStep.Code(page), "Studio", "abc#def", null, {}, {}, { submitted += 1 }, {}, {}) }) { root, _ ->
            val finish = tagged(root, "claude-finish")!!
            assertFalse("enabled", finish.config.contains(SemanticsProperties.Disabled))
            press(root, "claude-finish")
        }
        assertEquals(1, submitted)
    }

    @Test
    fun `a refused code offers Try Again, which starts again`() {
        var restarted = 0
        shown({
            ClaudeSignInSheet(ClaudeSignInStep.Failed(ClaudeSignIn.Failure.Refused, page), "Studio", "", null, {}, {}, {}, { restarted += 1 }, {})
        }) { root, said ->
            assertTrue(said.toString(), ClaudeSignIn.Failure.Refused.sentence in said)
            assertTrue(said.toString(), "Try again" in said)
            press(root, "claude-restart")
        }
        assertEquals(1, restarted)
    }

    @Test
    fun `the paste note is supporting text, not an error`() {
        shown({ ClaudeSignInSheet(ClaudeSignInStep.Code(page), "Studio", "abc#def", ClaudeSignIn.CODE_TOO_LONG, {}, {}, {}, {}, {}) }) { root, said ->
            assertTrue(said.toString(), ClaudeSignIn.CODE_TOO_LONG in said)
            assertEquals(null, tagged(root, "claude-code-error"))
            assertTrue(tagged(root, "claude-code-note") != null)
        }
    }

    @Test
    fun `Done closes, Cancel closes`() {
        var closed = 0
        shown({ ClaudeSignInSheet(ClaudeSignInStep.Done("alex@example.com"), "Studio", "", null, {}, {}, {}, {}, { closed += 1 }) }) { root, said ->
            assertTrue(said.toString(), "Signed in as" in said)
            assertEquals("no Cancel once done", null, tagged(root, "claude-cancel"))
            press(root, "claude-done")
        }
        shown({ ClaudeSignInSheet(ClaudeSignInStep.OpenPage(page), "Studio", "", null, {}, {}, {}, {}, { closed += 1 }) }) { root, _ ->
            press(root, "claude-cancel")
        }
        assertEquals(2, closed)
    }

    @Test
    fun `the page is checked again before it opens`() {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                val started = org.robolectric.Shadows.shadowOf(activity)
                assertEquals(PageOpen.REFUSED, com.farcooler.ui.openSignInPage(activity, "https://evil.example/cai/oauth/authorize"))
                assertEquals(PageOpen.REFUSED, com.farcooler.ui.openSignInPage(activity, "https://claude.com@evil.example/cai/oauth/authorize"))
                assertEquals("nothing launched for a page that isn't Claude's", null, started.nextStartedActivity)
                assertEquals(PageOpen.OPENED, com.farcooler.ui.openSignInPage(activity, page))
                assertEquals(android.net.Uri.parse(page), started.nextStartedActivity.data)
            }
        }
    }

    private fun screen(runner: String, content: @androidx.compose.runtime.Composable () -> Unit) = @androidx.compose.runtime.Composable {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp)) { content() }
    }

    @Test fun sectionNotSignedIn() = Capture.both("claude-section-none") {
        screen("Studio") { ClaudeSection(ClaudeLogin.NONE, "Studio", onSignIn = {}, onSignOut = {}) }()
    }

    @Test fun sectionNotSignedInLongRunner() = Capture.both("claude-section-none-long") {
        screen(longRunner) { ClaudeSection(ClaudeLogin.NONE, longRunner, onSignIn = {}, onSignOut = {}) }()
    }

    @Test fun sectionSignedInLongEmail() = Capture.both("claude-section-signed-in-long") {
        screen(longRunner) { ClaudeSection(ClaudeLogin(ClaudeLoginState.SIGNED_IN, email), longRunner, onSignIn = {}, onSignOut = {}) }()
    }

    @Test fun sectionStoppedWorking() = Capture.both("claude-section-stopped") {
        screen("Studio") { ClaudeSection(ClaudeLogin(ClaudeLoginState.STOPPED_WORKING, ""), "Studio", onSignIn = {}, onSignOut = {}) }()
    }

    // Starting and Finishing draw a spinner, whose endless animation never lets
    // Robolectric's looper go idle, so they are not drawn here.
    private fun sheet(name: String, step: ClaudeSignInStep, runner: String = "Studio", code: String = "", note: String? = null) =
        Capture.both(name) {
            // The sheet scrolls itself; a scroll around it would hand it no height.
            Column(Modifier.fillMaxSize().padding(top = 24.dp)) { ClaudeSignInSheet(step, runner, code, note, {}, {}, {}, {}, {}) }
        }

    @Test fun sheetOpenPage() = sheet("claude-sheet-open", ClaudeSignInStep.OpenPage(page))
    @Test fun sheetCodeEmpty() = sheet("claude-sheet-code", ClaudeSignInStep.Code(page))
    @Test fun sheetCodeLong() = sheet(
        "claude-sheet-code-long", ClaudeSignInStep.Code(page), longRunner,
        "ZXhhbXBsZS1hdXRob3JpemF0aW9uLWNvZGUtbm90LXJlYWwtb25seS1hLWNhcHR1cmU#c3RhdGUtdmFsdWUtZm9yLXRoZS1jYXB0dXJl",
    )
    @Test fun sheetDone() = sheet("claude-sheet-done", ClaudeSignInStep.Done("alex@example.com"))
    @Test fun sheetIncomplete() = sheet("claude-sheet-incomplete", ClaudeSignInStep.Failed(ClaudeSignIn.Failure.Incomplete, page), code = "abcdef")
    @Test fun sheetRefused() = sheet("claude-sheet-refused", ClaudeSignInStep.Failed(ClaudeSignIn.Failure.Refused, page))
    @Test fun sheetExpired() = sheet("claude-sheet-expired", ClaudeSignInStep.Failed(ClaudeSignIn.Failure.Expired, page))
    @Test fun sheetTooOld() = sheet("claude-sheet-too-old", ClaudeSignInStep.Failed(ClaudeSignIn.Failure.TooOld(longRunner), null), longRunner)
    @Test fun sheetSilent() = sheet("claude-sheet-silent", ClaudeSignInStep.Failed(ClaudeSignIn.Failure.Silent(longRunner), null), longRunner)
    @Test fun sheetDoneLong() = sheet("claude-sheet-done-long", ClaudeSignInStep.Done(email), longRunner)
    @Test fun sheetNoBrowser() = sheet("claude-sheet-no-browser", ClaudeSignInStep.Failed(ClaudeSignIn.Failure.NoBrowser, page))
    @Test fun sheetGeneric() = sheet("claude-sheet-generic", ClaudeSignInStep.Failed(ClaudeSignIn.Failure.Generic, null))
    @Test fun sheetTooOften() = sheet("claude-sheet-too-often", ClaudeSignInStep.Failed(ClaudeSignIn.Failure.TooOften, null))
    @Test fun sheetTooLong() = sheet("claude-sheet-too-long", ClaudeSignInStep.Code(page), code = "abc#def", note = ClaudeSignIn.CODE_TOO_LONG)

    @Test fun sectionSignOutFailed() = Capture.both("claude-section-sign-out-failed") {
        screen("Studio") { ClaudeSection(signedIn, "Studio", notice = ClaudeSignIn.SIGN_OUT_FAILED, onSignIn = {}, onSignOut = {}) }()
    }
}
