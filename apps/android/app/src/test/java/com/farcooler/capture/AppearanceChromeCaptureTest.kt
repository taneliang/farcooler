package com.farcooler.capture

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.farcooler.core.TerminalGrid
import com.farcooler.core.TerminalPalette
import com.farcooler.data.Theme
import com.farcooler.data.TerminalFontChoice
import com.farcooler.data.Themes
import com.farcooler.ui.Pane
import com.farcooler.ui.AppearanceSettings
import com.farcooler.ui.TerminalCanvas
import com.farcooler.ui.TerminalTabStrip
import com.farcooler.ui.WorktreeTopBar
import com.farcooler.ui.chromeGround
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The Settings rows for Appearance and Theme, and the worktree's chrome around a
 * terminal grid, under Light over a dark theme, Dark over a light theme and
 * Theme (ov-690). The grid is the app's own `TerminalCanvas` over a grid built
 * from the theme's colors, inside the bar and strip the worktree screen uses, so
 * what meets at the seam is what meets on a phone. Pictures only: they go to
 * `-Pfarcooler.captureDir`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class AppearanceChromeCaptureTest {
    /** Themes is process-wide state: a test that picks one must hand back the one it found, or the next class draws on it. */
    private var picked: String? = null

    @org.junit.Before
    fun rememberTheme() { picked = Themes.selected.value }

    @org.junit.After
    fun restoreTheme() { picked?.let { Themes.select(it) } }

    private val lines = listOf(
        "~/Dev/overnight on  main",
        "\$ git status --short",
        " M apps/android/app/src/main/java/com/farcooler/ui/Theme.kt",
        "?? apps/android/app/src/test/java/com/farcooler/capture/AppearanceViewTest.kt",
        "\$ ./gradlew testInstrumentedUnitTest assembleDebug --max-workers=2 -Pfarcooler.captures",
        "BUILD SUCCESSFUL in 41s",
        "\$ ",
    )

    private fun grid(columns: Int, rows: Int): TerminalGrid {
        val theme = Themes.current
        val cells = columns * rows
        val characters = IntArray(cells)
        lines.forEachIndexed { row, line ->
            line.take(columns).forEachIndexed { column, ch -> characters[row * columns + column] = ch.code }
        }
        return TerminalGrid(
            columns, rows, cursorRow = lines.size - 1, cursorColumn = 2, cursorVisible = true,
            displayOffset = 0, historySize = 0,
            characters = characters,
            foregrounds = IntArray(cells) { theme.foreground },
            backgrounds = IntArray(cells) { theme.background },
            flags = IntArray(cells),
        )
    }

    @Composable
    private fun Chrome() = Column(Modifier.fillMaxSize().background(chromeGround())) {
        WorktreeTopBar(
            worktree = null, fallbackTitle = "android-690", showRunner = false, runnerLabel = "", onOpenDrawer = {},
        )
        Box(Modifier.weight(1f)) {
            TerminalCanvas(
                grid = grid(columns = 44, rows = 30),
                fontFamily = FontFamily.Monospace, fontSize = 13f,
                onSize = { _, _ -> }, onTap = {}, onLongPress = { _, _ -> }, onScroll = { _, _, _ -> },
            )
        }
        TerminalTabStrip(worktree = null, counts = null, current = Pane.Changes, onSelect = {})
    }

    @Test
    fun worktreeChromeAroundTheGrid() {
        Capture.appearanceLooks("appearance-chrome") { Chrome() }
    }

    @Test
    fun settingsAppearanceAndTheme() {
        Capture.appearanceLooks("appearance-settings") {
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
                AppearanceSettings(
                    appearance = com.farcooler.ui.LocalAppearance.current,
                    onAppearance = {},
                    themes = Themes.available.value,
                    selectedTheme = Themes.current.name,
                    onTheme = {},
                    font = TerminalFontChoice.IOSEVKA,
                    onFont = {},
                    fontSize = 13f,
                    onFontSize = {},
                )
            }
        }
    }
}
