package com.farcooler.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import com.farcooler.R
import com.farcooler.data.Themes
import com.farcooler.data.TerminalFontChoice

/**
 * Whichever way the chosen theme goes — and Material You inside that.
 *
 * This was dark unconditionally, and the reasoning was sound as far as it went:
 * half the app is a terminal, a terminal is dark whatever the device is set to,
 * and a light list handing off to a black screen looked like two applications.
 * Choosing one beat reconciling two.
 *
 * What it could not survive is a light TERMINAL. Now that the palette is a
 * choice, the same argument points at following it: the chrome and the grid go
 * the same way because they are one theme, which is exactly the join that had
 * to be protected. Still one decision, just no longer a constant.
 *
 * Material You survives for the dark case. Android hands every app a palette
 * derived from the wallpaper, and an app that declines it reads as ported
 * rather than native — accents around a dark terminal do not fight it. A light
 * theme takes its own colours instead: picking Solarized Light and getting
 * wallpaper-derived dark surfaces would be the app ignoring what it was just
 * told.
 *
 * [isSystemInDarkTheme] is still deliberately not consulted. The theme decides,
 * and that is a preference this app owns rather than one it inherits.
 */
@Composable
fun FarCoolerTheme(content: @Composable () -> Unit) {
    val context = LocalContext.current
    // Recomposed when the theme changes, which is what carries a pick in
    // settings out to every surface in the app rather than just the terminal.
    val revision by Themes.revision.collectAsStateWithLifecycle()
    val scheme = remember(context, revision) {
        val theme = Themes.current
        // Material You survives, for the DARK case only.
        //
        // Wallpaper-derived colour is the platform-native behaviour this file
        // argues for at length, and it is still right when the chosen theme is
        // dark — the accents sit around a terminal, they do not fight it. It
        // yields entirely once a LIGHT theme is chosen: picking Solarized
        // Light and getting wallpaper-derived dark surfaces would be the app
        // ignoring what it was just told.
        val base =
            if (theme.dark) dynamicDarkColorScheme(context)
            else dynamicLightColorScheme(context)
        base.copy(
            // The one colour that is not the platform's to choose: it is
            // shared with the Mac and iOS so the same terminal looks like the
            // same terminal on all three.
            surfaceContainerLowest = Color(Themes.opaque(theme.background)),
        )
    }

    MaterialTheme(
        colorScheme = scheme,
        typography = MaterialTheme.typography,
        content = content,
    )
}

/**
 * The terminal's typeface.
 *
 * Not a free-form picker: the terminal draws a fixed grid of one glyph per
 * cell, and anything not genuinely monospaced would misalign the exact thing a
 * terminal is. Iosevka is bundled for the box-drawing and powerline glyphs
 * coding agents print constantly; the system's monospace is the fallback that
 * needs no bundle to have shipped correctly.
 */
object TerminalFonts {
    val iosevka = FontFamily(
        Font(R.font.iosevka_regular, FontWeight.Normal),
        Font(R.font.iosevka_bold, FontWeight.Bold),
    )

    fun family(choice: TerminalFontChoice): FontFamily = when (choice) {
        TerminalFontChoice.IOSEVKA -> iosevka
        TerminalFontChoice.SYSTEM -> FontFamily.Monospace
    }
}

/**
 * The dot colour for "is the process alive" — shared by the fleet list and the
 * terminal tab strip, so the same terminal cannot read green in one screen and
 * red in the other.
 */
@Composable
fun processColor(kind: com.farcooler.model.StateKind): Color = when (kind) {
    com.farcooler.model.StateKind.RUNNING -> Color(0xFF4CAF50)
    com.farcooler.model.StateKind.STARTING -> Color(0xFFFFC107)
    com.farcooler.model.StateKind.EXITED -> MaterialTheme.colorScheme.onSurfaceVariant
    // The one state that means Far Cooler does not know what happened.
    com.farcooler.model.StateKind.LOST, com.farcooler.model.StateKind.ERROR ->
        MaterialTheme.colorScheme.error
    com.farcooler.model.StateKind.UNKNOWN -> MaterialTheme.colorScheme.onSurfaceVariant
}

/**
 * The colour behind an agent's activity glyph, shared with the tab strip for
 * the same reason as [processColor]. Only the two states worth acting on get
 * colour, so a list of twenty still reads at a glance.
 */
@Composable
fun attentionColor(agent: com.farcooler.model.AgentActivity): Color = when (agent) {
    com.farcooler.model.AgentActivity.BLOCKED -> Color(0xFFFF9800)
    com.farcooler.model.AgentActivity.DONE -> Color(0xFF4CAF50)
    else -> MaterialTheme.colorScheme.onSurfaceVariant
}
