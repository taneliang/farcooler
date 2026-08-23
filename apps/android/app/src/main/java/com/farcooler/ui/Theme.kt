package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import com.farcooler.R
import com.farcooler.data.Themes
import com.farcooler.data.TerminalFontChoice
import androidx.compose.material3.Text
import com.farcooler.model.AgentActivity
import com.farcooler.model.InboxRow
import com.farcooler.model.StateKind
import com.farcooler.model.Terminal

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
 * terminal tab strip, so the same terminal cannot read one way in one screen
 * and another way in the other.
 *
 * **Silence is the default, and green is spent once.** This was green for
 * RUNNING, which put a green dot on every row of a list where green already
 * meant something else: [attentionColor] gives it to DONE, so an idle `zsh`
 * and an agent that had just finished its work wore the same mark, and on a
 * busy worktree they wore it three rows apart. A running process is the
 * ordinary case and now says nothing at all; green survives on exactly one
 * state, the same one the Mac spends it on. See `StatusGlyph` over there,
 * whose `.idle, .running, .exited` branch draws `Color.clear`. [ProcessDot]
 * reserves the column either way, so names line up down the list rather than
 * stepping in and out as panes start and stop.
 *
 * **Not amber for STARTING.** A pane is starting for well under a second, and
 * a colour nobody has time to read is a colour spent for nothing. It was worse
 * than spent here: Material amber 500 (`FFC107`) sat one step from the orange
 * 500 (`FF9800`) that means an agent is waiting on you, on an 8dp dot, so the
 * two most different pieces of news this list carries — "give it a moment" and
 * "come here now" — were told apart by a hue shift nobody can resolve at that
 * size. Starting is in progress, which is what the neutral means here and on
 * the Mac.
 *
 * **EXITED keeps its dot**, where the Mac draws nothing for it. The Mac can
 * afford the silence because its fused `Status` puts "Exited" on the row in
 * words; [Terminal.rowStatus] spells the state out only for panes that are NOT
 * agents, so an agent pane whose process is gone would otherwise lose its only
 * mark.
 *
 * That last sentence used to read "this row's supporting line says the process
 * state for every pane", and it was true when the fleet row's second line was
 * `terminal.state.lowercase()`. That line is now the host's signal line and the
 * state word survives only where there is no agent to describe instead — so the
 * argument for keeping this dot got STRONGER, not weaker, and the sentence that
 * made it had gone stale. iOS's copy of this comment has been the corrected
 * version since it rebuilt its own row.
 */
@Composable
fun processColor(kind: StateKind): Color = when (kind) {
    StateKind.RUNNING -> Color.Transparent
    StateKind.STARTING, StateKind.EXITED -> MaterialTheme.colorScheme.onSurfaceVariant
    // The one state that means Far Cooler does not know what happened.
    StateKind.LOST, StateKind.ERROR -> MaterialTheme.colorScheme.error
    // Not an error: the runner did not answer, which is a claim about the
    // reading and not about the pane. Painting the whole fleet red every time
    // tmux is busy is how a colour stops meaning anything.
    StateKind.UNKNOWN -> MaterialTheme.colorScheme.onSurfaceVariant
}

/** The one diameter, named so nobody has to pick a number at a call site. */
val PROCESS_DOT = 8.dp

/**
 * That dot, drawn — one shape, one size, and a hollow one where something is
 * missing.
 *
 * **Shape before colour.** Hollow says "something is not there" before the hue
 * does, which is the half of this vocabulary that survives a colourblind
 * reader, a greyscale screenshot and a phone in bright sun. The phones had
 * dropped it entirely and drew every state as a filled disc, leaving hue as
 * the only channel; `StatusGlyph.mark` on the Mac has carried it all along,
 * with the same three states hollow — a pane that died where the app was not
 * looking, one that failed to start, and one the runner would not report on.
 *
 * **One size.** The tab strip drew 6dp and the fleet list drew 8dp for the
 * same mark. That is the drift the Mac had to be pulled back from, where
 * eleven call sites passed five diameters of a glyph whose whole argument is
 * that it is always the same mark.
 */
@Composable
fun ProcessDot(kind: StateKind, modifier: Modifier = Modifier) {
    val colour = processColor(kind)
    val hollow = kind == StateKind.LOST || kind == StateKind.ERROR || kind == StateKind.UNKNOWN
    Box(
        modifier
            .size(PROCESS_DOT)
            .then(
                if (hollow) Modifier.border(1.5.dp, colour, CircleShape)
                // A transparent fill still claims the box, which is the point:
                // the column is reserved whether or not anything occupies it.
                else Modifier.clip(CircleShape).background(colour)
            )
    )
}


/**
 * The colour behind an agent's activity glyph, shared with the tab strip for
 * the same reason as [processColor]. Only the two states worth acting on get
 * colour, so a list of twenty still reads at a glance.
 */
@Composable
fun attentionColor(agent: AgentActivity): Color = when (agent) {
    AgentActivity.BLOCKED -> Color(0xFFFF9800)
    AgentActivity.DONE -> Color(0xFF4CAF50)
    else -> MaterialTheme.colorScheme.onSurfaceVariant
}

/**
 * The same colour, for a terminal whose finished turn may have DIED.
 *
 * Green and red are the whole difference between "it's done" and "it stopped
 * working", and [AgentActivity] alone cannot tell them apart — the daemon
 * sends both as `done` and says which in `turnFailed`. This app decoded no such
 * field, so an agent whose turn had died wore the green checkmark of one that
 * had succeeded: the second place green meant two things on this screen, and
 * the one that mattered more. iOS has had this overload since the field
 * landed; see `attentionColor(_ terminal:)` in its `FleetView`.
 */
@Composable
fun attentionColor(terminal: Terminal): Color =
    attentionColor(terminal.agent, terminal.turnDidFail)

/**
 * The same rule again, for a surface that has already taken the two facts apart
 * — `TerminalTabStrip` flattens its terminals into chips before it draws them.
 * One rule in one place either way: a chip's ring and the row it names must not
 * be able to come out different colours.
 */
@Composable
fun attentionColor(agent: AgentActivity, turnDidFail: Boolean): Color =
    if (turnDidFail) MaterialTheme.colorScheme.error else attentionColor(agent)

/**
 * `+82 -13`: how much a worktree has changed, in the two colours those signs
 * have everywhere in this app.
 *
 * One copy, not one per surface. The front door's review row wrote this out and
 * the workspace's Changes chip was about to write it out again, which is the
 * shape `df87410` already had to pull the landing ordering back from: three
 * copies of one rule is three chances for a phone to disagree with itself.
 *
 * The green is a literal and matches [attentionColor]'s green to the byte,
 * because it is the one colour Material's scheme has no role for — there is no
 * "positive" slot in a `ColorScheme` the way there is an `error` one. It is
 * written here rather than borrowed from `attentionColor` so that a change to
 * what a FINISHED AGENT looks like cannot silently change what an added line
 * looks like. Red IS the scheme's error role, because "removed" and "went
 * wrong" want the same red under every theme.
 */
@Composable
fun DiffCounts(counts: InboxRow, modifier: Modifier = Modifier) {
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(
            "+${counts.insertions}",
            style = MaterialTheme.typography.labelSmall,
            fontFamily = FontFamily.Monospace,
            color = DIFF_ADDED,
        )
        Text(
            "-${counts.deletions}",
            style = MaterialTheme.typography.labelSmall,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.error,
        )
    }
}

/** The green a diff's insertions are drawn in. See [DiffCounts]. */
private val DIFF_ADDED = Color(0xFF4CAF50)
