package com.farcooler.capture

import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.Composable
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import com.farcooler.data.Appearance
import com.farcooler.data.Settings
import com.farcooler.data.Theme
import com.farcooler.data.Themes
import com.farcooler.ui.FarCoolerTheme
import androidx.compose.material3.MaterialTheme
import com.farcooler.ui.LocalGlanceDark
import com.farcooler.ui.cardColor
import com.farcooler.ui.chromeGround
import com.farcooler.ui.YourMessage
import com.farcooler.ui.LocalHighContrast
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowLooper

/**
 * The Appearance setting's effect on what the app draws (ov-690): System follows
 * the phone, live; Light and Dark override it; Theme is the old rule. Run only
 * under `-Pfarcooler.captures`, which is JDK 21's.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class AppearanceViewTest {
    /** Themes is process-wide state: a test that picks one must hand back the one it found, or the next class draws on it. */
    private var picked: String? = null

    @org.junit.Before
    fun rememberTheme() { picked = Themes.selected.value }

    @org.junit.After
    fun restoreTheme() { picked?.let { Themes.select(it) } }

    /**
     * Building a `Settings` installs the process-wide image stores (`PaneImageDisk`) on that test's
     * temporary directories. Left there, the next class's pictures are read from a directory
     * Robolectric has deleted, which is what failed
     * `NativePolishCaptureTest.noThumbnailsAreDrawnWhereTheRunnerServesNoImages` in a full run.
     */
    @org.junit.After
    fun forgetImageStores() {
        com.farcooler.net.PaneImageDisk.drafts = null
        com.farcooler.net.PaneImageDisk.prompts = null
    }

    private val light = Theme("Appearance Light", dark = false, 0xFAFAFA, 0x24292F, 0x24292F, List(16) { 0x808080 })

    private fun pick(terminalDark: Boolean) {
        Themes.merge(listOf(light), "appearance-test")
        Themes.select(if (terminalDark) "Nord" else light.name)
    }

    /** Whether the glance vocabulary, and so the bubble, resolved dark; `systemDark` null reads the phone. */
    private fun resolvedDark(appearance: Appearance, systemDark: Boolean?, terminalDark: Boolean): Boolean {
        pick(terminalDark)
        var seen: Boolean? = null
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                activity.setContent {
                    val content: @Composable () -> Unit = { seen = LocalGlanceDark.current }
                    if (systemDark == null) FarCoolerTheme(appearance = appearance, content = content)
                    else FarCoolerTheme(appearance = appearance, systemDark = systemDark, content = content)
                }
            }
            ShadowLooper.idleMainLooper()
        }
        return seen!!
    }

    @Test
    fun `System follows the phone whatever the terminal theme is`() {
        for (system in listOf(false, true)) for (terminal in listOf(false, true)) {
            assertEquals("system=$system terminal=$terminal", system, resolvedDark(Appearance.SYSTEM, system, terminal))
        }
    }

    @Test
    fun `Light and Dark override the phone and the terminal theme`() {
        for (system in listOf(false, true)) for (terminal in listOf(false, true)) {
            assertEquals(false, resolvedDark(Appearance.LIGHT, system, terminal))
            assertEquals(true, resolvedDark(Appearance.DARK, system, terminal))
        }
    }

    @Test
    fun `Theme keeps the old rule`() {
        for (system in listOf(false, true)) for (terminal in listOf(false, true)) {
            assertEquals(terminal, resolvedDark(Appearance.THEME, system, terminal))
        }
    }

    @Test
    @Config(qualifiers = "w411dp-h891dp-night-xxhdpi")
    fun `System reads the phone's night mode when nobody passes one`() {
        assertEquals(true, resolvedDark(Appearance.SYSTEM, null, terminalDark = false))
    }

    @Test
    @Config(qualifiers = "w411dp-h891dp-notnight-xxhdpi")
    fun `System reads the phone's day mode when nobody passes one`() {
        assertEquals(false, resolvedDark(Appearance.SYSTEM, null, terminalDark = true))
    }

    @Test
    fun `System follows the phone when it flips while the app is open`() {
        pick(terminalDark = true)
        val seen = mutableListOf<Boolean>()
        val phone = androidx.compose.runtime.mutableStateOf(false)
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                activity.setContent {
                    FarCoolerTheme(appearance = Appearance.SYSTEM, systemDark = phone.value) { seen += LocalGlanceDark.current }
                }
            }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { phone.value = true }
            ShadowLooper.idleMainLooper()
        }
        assertEquals(listOf(false, true), seen.distinct())
    }

    @Test
    fun `the bubble takes the resolved mode and high contrast still wins`() {
        assertEquals(YourMessage.light, YourMessage.fill(resolvedDark(Appearance.LIGHT, true, true), highContrast = false))
        assertEquals(YourMessage.dark, YourMessage.fill(resolvedDark(Appearance.DARK, false, false), highContrast = false))
        pick(terminalDark = false)
        var contrast: Boolean? = null
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme(contrast = 1f, appearance = Appearance.LIGHT) { contrast = LocalHighContrast.current } } }
            ShadowLooper.idleMainLooper()
        }
        assertEquals(true, contrast)
    }

    @Test
    fun `the choice persists and starts as System`() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        context.getSharedPreferences("farcooler.settings", android.content.Context.MODE_PRIVATE).edit().clear().commit()
        assertEquals(Appearance.SYSTEM, Settings(context).appearance.value)
        Settings(context).setAppearance(Appearance.DARK)
        assertEquals(Appearance.DARK, Settings(context).appearance.value)
        Settings(context).setAppearance(Appearance.LIGHT)
        assertEquals(Appearance.LIGHT, Settings(context).appearance.value)
    }

    /** What [content] reads inside `FarCoolerTheme(appearance)`, with the terminal theme dark or light. */
    private fun <T> read(appearance: Appearance, terminalDark: Boolean, content: @Composable () -> T): T {
        pick(terminalDark)
        var out: T? = null
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { it.setContent { FarCoolerTheme(appearance = appearance, systemDark = false) { out = content() } } }
            ShadowLooper.idleMainLooper()
        }
        @Suppress("UNCHECKED_CAST")
        return out as T
    }

    @Test
    fun `chromeGround is the terminal's under Theme and the scheme's under the rest`() {
        for (terminal in listOf(false, true)) {
            val theme = read(Appearance.THEME, terminal) { chromeGround() to com.farcooler.core.TerminalPalette.BACKGROUND }
            assertEquals(androidx.compose.ui.graphics.Color(theme.second), theme.first)
            for (appearance in listOf(Appearance.SYSTEM, Appearance.LIGHT, Appearance.DARK)) {
                val (ground, scheme) = read(appearance, terminal) { chromeGround() to MaterialTheme.colorScheme.background }
                assertEquals("$appearance over terminalDark=$terminal", scheme, ground)
            }
        }
    }

    @Test
    fun `a diff card's wash follows the app's mode and not the terminal theme`() {
        val white = androidx.compose.ui.graphics.Color.White.copy(alpha = 0.06f)
        val black = androidx.compose.ui.graphics.Color.Black.copy(alpha = 0.04f)
        // Light chrome over a dark terminal theme, and the other way round.
        assertEquals(black, read(Appearance.LIGHT, terminalDark = true) { cardColor() })
        assertEquals(white, read(Appearance.DARK, terminalDark = false) { cardColor() })
        assertEquals(white, read(Appearance.THEME, terminalDark = true) { cardColor() })
        assertEquals(black, read(Appearance.THEME, terminalDark = false) { cardColor() })
    }

    @Test
    fun `System follows the phone's own night mode when it changes under the app`() {
        pick(terminalDark = true)
        // MainActivity handles `uiMode` itself (AndroidManifest.xml), so the activity is told
        // and not rebuilt; say the same of the stand-in, or its content would be lost.
        val app = ApplicationProvider.getApplicationContext<android.app.Application>()
        org.robolectric.Shadows.shadowOf(app.packageManager).addOrUpdateActivity(
            android.content.pm.ActivityInfo().apply {
                name = ComponentActivity::class.java.name
                packageName = app.packageName
                configChanges = android.content.pm.ActivityInfo.CONFIG_UI_MODE
            },
        )
        val seen = mutableListOf<Boolean>()
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                // No `systemDark` passed: this is `isSystemInDarkTheme()` reading the configuration.
                activity.setContent { FarCoolerTheme(appearance = Appearance.SYSTEM) { seen += LocalGlanceDark.current } }
            }
            ShadowLooper.idleMainLooper()
            org.robolectric.RuntimeEnvironment.setQualifiers("+night")
            ShadowLooper.idleMainLooper()
        }
        assertEquals(listOf(false, true), seen.distinct())
    }
}
