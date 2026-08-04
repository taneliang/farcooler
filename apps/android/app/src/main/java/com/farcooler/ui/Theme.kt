package com.farcooler.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import com.farcooler.R
import com.farcooler.core.TerminalPalette
import com.farcooler.data.TerminalFontChoice

/**
 * Dark, always — and Material You inside that.
 *
 * The Apple apps force dark app-wide, and the reasoning transfers exactly: half
 * the app is a terminal, a terminal is dark whatever the device is set to, and
 * a light list handing off to a black screen and back looked like two
 * applications. Choosing one is better than reconciling two.
 *
 * What does not transfer is ignoring the platform's own colour. Android hands
 * every app a palette derived from the wallpaper, and an app that declines it
 * reads as ported rather than native. So the scheme is the system's DARK
 * dynamic palette where the device offers one — accents, containers and
 * surfaces all from the user's wallpaper — with only the terminal's own
 * background pinned, because that colour is shared with the Mac and iOS apps so
 * the same terminal looks like the same terminal on all three.
 *
 * [isSystemInDarkTheme] is deliberately not consulted. It is the one place this
 * app overrules the system, and it does so for a reason that does not depend on
 * a preference.
 */
@Composable
fun FarCoolerTheme(content: @Composable () -> Unit) {
    val context = LocalContext.current
    val scheme = remember(context) {
        dynamicDarkColorScheme(context).copy(
            // The one colour that is not the platform's to choose.
            surfaceContainerLowest = Color(TerminalPalette.BACKGROUND),
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
