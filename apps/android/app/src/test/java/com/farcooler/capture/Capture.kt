package com.farcooler.capture

import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier
import androidx.core.view.drawToBitmap
import androidx.test.core.app.ActivityScenario
import com.farcooler.data.Theme
import com.farcooler.data.Themes
import com.farcooler.ui.FarCoolerTheme
import com.github.takahirom.roborazzi.captureRoboImage
import java.io.File
import org.robolectric.shadows.ShadowLooper

/**
 * Draws a composable offscreen, in the app's own theme, light and dark, and
 * writes each as a PNG for a person to look at (ov-245).
 *
 * It records and compares nothing: there is no stored image to drift from, so
 * no golden-image gate. The pictures go to `-Pfarcooler.captureDir`.
 *
 * Roborazzi's own `onView` path and Compose's test rule both go through
 * Espresso, whose input injection reaches for `InputManager.getInstance()`,
 * gone on SDK 37 (this app's `minSdk`). So the activity is launched with
 * `ActivityScenario`, the main looper is idled by hand, and the decor view is
 * drawn to a bitmap that Roborazzi writes.
 *
 * The app picks light or dark itself (`Themes`), not the system, so each pass
 * selects a theme. Colors are Robolectric's default accent, not a wallpaper's:
 * `dynamicLightColorScheme` reads the platform's, as a phone does.
 */
object Capture {
    private val lightTheme = Theme(
        name = "Capture Light",
        dark = false,
        background = 0xFAFAFA,
        foreground = 0x24292F,
        cursor = 0x24292F,
        ansi = List(16) { 0x808080 },
    )

    private val outDir: File
        get() = File(System.getProperty("farcooler.captureDir") ?: "build/captures").also { it.mkdirs() }

    /** `<name>-light.png` and `<name>-dark.png`. */
    fun both(name: String, content: @androidx.compose.runtime.Composable () -> Unit) {
        Themes.merge(listOf(lightTheme), "capture")
        for ((mode, theme) in listOf("light" to lightTheme.name, "dark" to "Nord")) {
            Themes.select(theme)
            one(File(outDir, "$name-$mode.png"), content)
        }
    }

    /**
     * [both], with the dialog [content] opens drawn over the screen where the
     * phone puts it: centered, over Material's scrim. A dialog is a window of
     * its own, which the screen's bitmap doesn't hold (ov-299).
     */
    fun bothWithDialog(name: String, content: @androidx.compose.runtime.Composable () -> Unit) {
        Themes.merge(listOf(lightTheme), "capture")
        for ((mode, theme) in listOf("light" to lightTheme.name, "dark" to "Nord")) {
            Themes.select(theme)
            one(File(outDir, "$name-$mode.png"), content, withDialog = true)
        }
    }

    private fun one(
        file: File,
        content: @androidx.compose.runtime.Composable () -> Unit,
        withDialog: Boolean = false,
    ) {
        ActivityScenario.launch(ComponentActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                activity.setContent {
                    FarCoolerTheme {
                        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) { content() }
                    }
                }
            }
            ShadowLooper.idleMainLooper()
            scenario.onActivity { activity ->
                val screen = activity.window.decorView.drawToBitmap()
                val dialog = if (withDialog) org.robolectric.shadows.ShadowDialog.getLatestDialog() else null
                if (dialog == null) {
                    screen.captureRoboImage(file.path)
                } else {
                    val card = dialog.window!!.decorView.drawToBitmap()
                    val canvas = android.graphics.Canvas(screen)
                    canvas.drawColor(android.graphics.Color.argb(82, 0, 0, 0))
                    canvas.drawBitmap(card, (screen.width - card.width) / 2f, (screen.height - card.height) / 2f, null)
                    screen.captureRoboImage(file.path)
                }
            }
        }
    }
}
