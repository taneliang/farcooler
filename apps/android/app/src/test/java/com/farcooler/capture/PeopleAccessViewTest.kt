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
import com.farcooler.model.PeopleAccess
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.PeopleAccessActions
import com.farcooler.ui.PeopleAccessSection
import com.farcooler.ui.RemoveDeviceDialog
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * The People screen's Access and the step-up behind its buttons (ov-856, slice 4): the app's own
 * `PeopleAccessSection`, its words, Remove Device asking what it takes, Allow pressed only on the same person's other
 * device, and the pictures. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class PeopleAccessViewTest {
    private val members = Json.parseToJsonElement(
        """{"members":[
          {"handle":"cosmo","name":"Cosmo","devices":["phone-c","pad-c"],"access":"member"},
          {"handle":"dan","name":"Dan","devices":["phone-d","mac-d"],"access":"admin"}],
          "claude_account":"","you":"",
          "device_access":[
            {"client_id":"phone-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
            {"client_id":"ipad-o","person":"","access":"owner","cap":"","check":false,"step_up":"proposed","step_up_code":"418207","step_up_at":9},
            {"client_id":"legacy-t","person":"","access":"owner","cap":"member","check":true,"step_up":"","step_up_code":"","step_up_at":0},
            {"client_id":"phone-c","person":"cosmo","access":"member","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
            {"client_id":"pad-c","person":"cosmo","access":"member","cap":"viewer","check":false,"step_up":"","step_up_code":"","step_up_at":0},
            {"client_id":"phone-d","person":"dan","access":"admin","cap":"","check":false,"step_up":"proposed","step_up_code":"730412","step_up_at":9},
            {"client_id":"mac-d","person":"dan","access":"admin","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5}]}""",
    ).jsonObject

    private val clients = Json.parseToJsonElement(
        """{"clients":[
          {"clientId":"phone-o","label":"farcooler-Pixel-aaaa1111","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"ipad-o","label":"farcooler-iPad-hhhh8888","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"legacy-t","label":"farcooler-Old-iPad-cccc3333","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"phone-c","label":"farcooler-Cosmos-iPhone-dddd4444","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"pad-c","label":"farcooler-Cosmos-iPad-eeee5555","shellAccess":false,"foreign":false,"addedBy":"phone-c"},
          {"clientId":"phone-d","label":"farcooler-Dans-iPhone-ffff6666","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"mac-d","label":"farcooler-Dans-Mac-gggg7777","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"mac-d","label":"farcooler-shell-Dans-Mac-gggg7777.mac-d","shellAccess":true,"foreign":false,"addedBy":""}]}""",
    ).jsonObject

    private val access = PeopleAccess.of(members, clients, "phone-o")!!

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
        if (node.config.getOrNull(SemanticsProperties.TestTag) == tag) node else node.children.firstNotNullOfOrNull { tagged(it, tag) }

    private fun everyRoot(): List<SemanticsNode> {
        val global = Class.forName("android.view.WindowManagerGlobal")
        val instance = global.getMethod("getInstance").invoke(null)
        val views = global.getDeclaredField("mViews").also { it.isAccessible = true }.get(instance) as List<*>
        return views.filterIsInstance<View>().flatMap { roots(it) }.map { it.semanticsOwner.unmergedRootSemanticsNode }
    }

    /** The merged trees, where a button and its label are one node. */
    private fun everyMergedRoot(): List<SemanticsNode> {
        val global = Class.forName("android.view.WindowManagerGlobal")
        val instance = global.getMethod("getInstance").invoke(null)
        val views = global.getDeclaredField("mViews").also { it.isAccessible = true }.get(instance) as List<*>
        return views.filterIsInstance<View>().flatMap { roots(it) }.map { it.semanticsOwner.rootSemanticsNode }
    }

    private fun screen(
        actions: PeopleAccessActions = PeopleAccessActions(),
        shownCode: String? = null,
        note: String? = null,
        on: PeopleAccess = access,
    ) = @androidx.compose.runtime.Composable {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
            PeopleAccessSection(on, "studio", actions, shownCode, note)
        }
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
    fun `each person says their access and their devices carry their warnings`() {
        shown(screen()) { root, said ->
            assertEquals("Member", (tagged(root, "people-access-cosmo")!!.config[SemanticsProperties.Text]).single().text)
            assertEquals("Admin", (tagged(root, "people-access-dan")!!.config[SemanticsProperties.Text]).single().text)
            for (line in listOf(
                "Added by Cosmos-iPhone", PeopleAccess.VIEWER_BADGE, "⚠ ${PeopleAccess.SHELL_BADGE}",
                "This device has a shell on studio, so it can do everything.", PeopleAccess.CHECK_HEADER, PeopleAccess.WAITING_HEADER,
            )) assertTrue("$line in $said", line in said)
            assertNotNull(tagged(root, "people-check"))
            assertNotNull(tagged(root, "people-waiting"))
        }
    }

    @Test
    fun `Allow is offered for the same persons other device and not for another persons`() {
        val offered = mutableListOf<String>()
        shown(screen(PeopleAccessActions(approve = { offered += it.id }))) { root, said ->
            assertTrue("Only allow this if you’re holding iPad right now. Check that it shows 418 207." in said)
            assertNull("Dan's key is Dan's to approve", tagged(root, "waiting-allow-phone-d"))
            tagged(root, "waiting-allow-ipad-o")!!.config[SemanticsActions.OnClick].action!!.invoke()
        }
        assertEquals(listOf("ipad-o"), offered)
    }

    @Test
    fun `the device menu offers Remove device`() {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme { screen(PeopleAccessActions(removeDevice = { _, _ -> }))() } } }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val root = roots(activity.window.decorView).first().semanticsOwner.unmergedRootSemanticsNode
                assertNull("closed until it is opened", everyRoot().firstNotNullOfOrNull { tagged(it, "device-remove-phone-c") })
                tagged(root, "device-menu-phone-c")!!.config[SemanticsActions.OnClick].action!!.invoke()
            }
            settle()
            scenario.onActivity {
                assertNotNull("the menu offers Remove device", everyRoot().firstNotNullOfOrNull { tagged(it, "device-remove-phone-c") })
            }
        }
    }

    @Test
    fun `Remove device asks what it takes before it asks the runner`() {
        val asked = mutableListOf<Boolean>()
        val phone = access.people.first { it.handle == "cosmo" }.devices[0]
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme { RemoveDeviceDialog(phone, "studio", onDismiss = {}) { asked += it } } } }
            settle()
            scenario.onActivity {
                val said = mutableListOf<String>().also { list -> everyRoot().forEach { root -> texts(root, list) } }
                assertTrue("$said", "Remove Cosmos-iPhone?" in said)
                assertTrue("$said", "Cosmos-iPhone loses its access to studio right away. It added one other device." in said)
                assertTrue("nothing is asked until a button is pressed", asked.isEmpty())
                val both = everyMergedRoot().firstNotNullOfOrNull { findText(it, "Remove device and 1 it added") }
                both!!.config[SemanticsActions.OnClick].action!!.invoke()
            }
            settle()
        }
        assertEquals("what it added goes too", listOf(true), asked)
    }

    /** Idle the main looper long enough for a menu to close and a dialog to open. */
    private fun settle() {
        repeat(3) { ShadowLooper.idleMainLooper(300, java.util.concurrent.TimeUnit.MILLISECONDS) }
    }

    private fun findText(node: SemanticsNode, text: String): SemanticsNode? {
        val own = node.config.getOrNull(SemanticsProperties.Text)?.any { it.text == text } == true
        if (own && node.config.contains(SemanticsActions.OnClick)) return node
        return node.children.firstNotNullOfOrNull { findText(it, text) }
    }

    @Test
    fun `this device shows Allow admin actions, then the digits to check`() {
        shown(screen(PeopleAccessActions(allowAdminActions = {}), on = PeopleAccess.of(members.withoutOwnKey(), clients, "phone-o")!!)) { root, said ->
            assertNotNull(tagged(root, "admin-actions-allow"))
            assertTrue("Allow admin actions" in said)
        }
        shown(screen(shownCode = "273905", on = PeopleAccess.of(members.withoutOwnKey(), clients, "phone-o")!!)) { root, said ->
            assertEquals("273 905", tagged(root, "admin-actions-code")!!.config[SemanticsProperties.Text].single().text)
            assertNull(tagged(root, "admin-actions-allow"))
            assertTrue("Approve on another device" in said)
        }
        shown(screen()) { root, _ -> assertNotNull("a trusted key says admin actions are on", tagged(root, "admin-actions-on")) }
    }

    /** The same runner with this phone's own key not yet trusted. */
    private fun JsonObject.withoutOwnKey(): JsonObject {
        val rows = (this["device_access"] as JsonArray).map { row ->
            val o = row.jsonObject
            if (o["client_id"]!!.jsonPrimitive.content == "phone-o") JsonObject(o + ("step_up" to JsonPrimitive(""))) else o
        }
        return JsonObject(this + ("device_access" to JsonArray(rows)))
    }

    @Test
    fun `the note is the sentence for a refusal`() {
        shown(screen(note = "Allow admin actions on this phone from your Mac first.")) { root, said ->
            assertNotNull(tagged(root, "people-access-note"))
            assertFalse(said.toString(), said.any { "step_up" in it })
        }
    }

    @Test fun peopleAccess() = Capture.both("people-access", content = screen(PeopleAccessActions(removeDevice = { _, _ -> }, approve = {}, allowAdminActions = {})))

    @Test fun stepUpPromptNote() = Capture.both(
        "people-allow-admin", content = screen(PeopleAccessActions(allowAdminActions = {}), shownCode = "273905", on = PeopleAccess.of(members.withoutOwnKey(), clients, "phone-o")!!),
    )
}
