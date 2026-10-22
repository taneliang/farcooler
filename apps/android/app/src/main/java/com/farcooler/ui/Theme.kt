package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.VerticalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
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
import com.farcooler.data.Appearance
import com.farcooler.data.Themes
import com.farcooler.data.TerminalFontChoice
import androidx.compose.material3.Text
import com.farcooler.model.InboxRow
import com.farcooler.model.StateKind
import com.farcooler.model.Terminal

/**
 * Light or dark for the app, from the Appearance setting (ov-690), and Material
 * You inside that.
 *
 * This was dark unconditionally, and the reasoning was sound as far as it went:
 * half the app is a terminal, a terminal is dark whatever the device is set to,
 * and a light list handing off to a black screen looked like two applications.
 * Then it followed the chosen terminal theme, for the same reason. What that
 * could not do is read as a chat: the conversation is most of what this app
 * shows, the iPhone and the Mac draw it in the system's mode, and a message
 * thread that ignores the phone's light mode is the odd one out.
 *
 * So [appearance] decides, as the Mac's does: System follows the phone live,
 * Light and Dark override it, and Theme keeps the old rule (the chosen terminal
 * theme decides) for whoever wants the chrome to match the grid. The terminal
 * itself never reads this: it draws `Themes.current`'s palette whatever the
 * chrome around it is. Settings and lists follow the same answer, as the Mac's
 * `NSApp.appearance` does for settings and sheets.
 *
 * Material You is used in either mode: the wallpaper's palette in the resolved
 * direction, with the terminal's own background pinned where the terminal shows.
 *
 * [contrast] is the system's contrast level (ov-624), which the app does follow:
 * it is an accessibility setting for the whole phone. It reaches a surface as
 * [LocalHighContrast].
 *
 * The default [Appearance.THEME] is for previews and tests drawn without a
 * setting; the app passes the stored one, which defaults to System.
 */
@Composable
fun FarCoolerTheme(
    contrast: Float = rememberSystemContrast(),
    appearance: Appearance = Appearance.THEME,
    systemDark: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    val context = LocalContext.current
    // Recomposed when the theme changes, which is what carries a pick in
    // settings out to every surface in the app rather than just the terminal.
    val revision by Themes.revision.collectAsStateWithLifecycle()
    // Which way the chosen theme goes, hoisted out of [scheme] because the
    // glance vocabulary needs the same answer and must not ask a second time.
    // §01's light palette is a different set of values rather than a filter over
    // the dark one, so "which palette" is a decision, and a decision made twice
    // is a decision two surfaces can make differently.
    val dark = remember(revision, appearance, systemDark) { appearance.dark(systemDark, Themes.current.dark) }
    val scheme = remember(context, revision, dark, appearance) {
        val theme = Themes.current
        val base =
            if (dark) dynamicDarkColorScheme(context)
            else dynamicLightColorScheme(context)
        // Under Theme the chrome IS the terminal's ground (see [chromeGround]), so
        // the one color that is not the platform's to choose is pinned to it, as
        // the Mac and iOS do for the same terminal. Under the other three the
        // chrome is the platform's, and so is this.
        if (appearance == Appearance.THEME) {
            base.copy(surfaceContainerLowest = Color(Themes.opaque(theme.background)))
        } else base
    }

    MaterialTheme(
        colorScheme = scheme,
        typography = MaterialTheme.typography,
    ) {
        // Inside, not outside: [glanceInk1] and [glanceInk2] fall back to the
        // scheme's own on-surface roles in light mode, so the glance vocabulary
        // has to be able to see the colors Material just resolved.
        ProvideGlanceAppearance(dark, highContrast = contrast > 0f) {
            CompositionLocalProvider(LocalAppearance provides appearance, content = content)
        }
    }
}

/** The Appearance setting in force, for the few surfaces that must know it is Theme. See [chromeGround]. */
val LocalAppearance: ProvidableCompositionLocal<Appearance> = staticCompositionLocalOf { Appearance.THEME }

/**
 * The ground the worktree screen's chrome sits on: the bar above a pane, the
 * strip below it, the Changes tab, and the band behind them.
 *
 * Under Theme it is the terminal's own background, as it has always been, so a
 * dark terminal continues up into its bar as one thing. Under System, Light and
 * Dark the chrome follows the app's mode like every other surface, and the
 * terminal grid is an island of the chosen theme inside it: the grid draws its
 * own background, so it keeps the theme whatever the chrome does (ov-690).
 * Text on this ground comes from the scheme, which is why the two must change
 * together and why this is one function rather than a color at each call site.
 */
@Composable
@ReadOnlyComposable
fun chromeGround(): Color =
    if (LocalAppearance.current == Appearance.THEME) Color(com.farcooler.core.TerminalPalette.BACKGROUND)
    else MaterialTheme.colorScheme.background

/**
 * The three corner steps, in the same points as the iOS and Mac `Radius`
 * (DesignTokens.swift in AgentKit), so a card is the same shape on every
 * screen the owner holds (ov-227). Steps, not numbers: a fourth value is a
 * design change made here, never in a view.
 *
 * A shape nested `padding` inside a parent's corner takes `parent - padding`,
 * never below [small], so it stays concentric with the parent.
 */
object Radius {
    /** Something the hand acts on inside a list or bar: a key, a chip, a code span. */
    val small = 6.dp

    /** A card: a message, a tool block, a note, a preview. */
    val medium = 10.dp

    /** A free-floating surface: the composer, an approval over the transcript, a bar. */
    val large = 16.dp
}

/**
 * The one rule a list or a sheet draws between two rows: one density-independent
 * pixel of the scheme's `outlineVariant`, which is what Material's own divider
 * already was, so naming it moves no pixel (ov-276). Where a rule is allowed is
 * decided here and not in a screen; prefer space between groups.
 */
@Composable
fun Separator(modifier: Modifier = Modifier) {
    HorizontalDivider(modifier, color = MaterialTheme.colorScheme.outlineVariant)
}

/** [Separator] turned to stand between side-by-side panes. */
@Composable
fun VerticalSeparator(modifier: Modifier = Modifier) {
    VerticalDivider(modifier, color = MaterialTheme.colorScheme.outlineVariant)
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
 * The dot color for "is the process alive" — shared by the fleet list and the
 * terminal tab strip, so the same terminal cannot read one way in one screen
 * and another way in the other.
 *
 * **Silence is the default, and green is spent once.** This was green for
 * RUNNING, which put a green dot on every row of a list where green already
 * meant something else: the agent mark gave it to DONE, so an idle `zsh`
 * and an agent that had just finished its work wore the same mark, and on a
 * busy worktree they wore it three rows apart. A running process is the
 * ordinary case and now says nothing at all; green survives on exactly one
 * state, the same one the Mac spends it on. See `StatusGlyph` over there,
 * whose `.idle, .running, .exited` branch draws `Color.clear`. [ProcessDot]
 * reserves the column either way, so names line up down the list rather than
 * stepping in and out as panes start and stop.
 *
 * **Not amber for STARTING.** A pane is starting for well under a second, and
 * a color nobody has time to read is a color spent for nothing. It was worse
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
    // tmux is busy is how a color stops meaning anything.
    StateKind.UNKNOWN -> MaterialTheme.colorScheme.onSurfaceVariant
}

/** The one diameter, named so nobody has to pick a number at a call site. */
val PROCESS_DOT = 8.dp

/**
 * That dot, drawn — one shape, one size, and a hollow one where something is
 * missing.
 *
 * **Shape before color.** Hollow says "something is not there" before the hue
 * does, which is the half of this vocabulary that survives a colorblind
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
    val color = processColor(kind)
    val hollow = kind == StateKind.LOST || kind == StateKind.ERROR || kind == StateKind.UNKNOWN
    Box(
        modifier
            .size(PROCESS_DOT)
            .then(
                if (hollow) Modifier.border(1.5.dp, color, CircleShape)
                // A transparent fill still claims the box, which is the point:
                // the column is reserved whether or not anything occupies it.
                else Modifier.clip(CircleShape).background(color)
            )
    )
}


/*
 * `attentionColor` was here, in three overloads, and the whole of it is gone.
 *
 * It gave Material orange 500 to a blocked agent, Material green 500 to a
 * finished one and the scheme's error red to a turn that died, and every one of
 * those was a color this file mixed by hand. §01 of the glance spec reserves
 * ONE saturated hue for "does this need me" and forbids any other loud thing;
 * three hues answering that one question is the failure the rule names. The
 * replacement is `model/Glance.kt` — `GlancePalette.amber` for the tier and
 * `GlanceMark` for the drawing — and it is shared with iOS and the Mac rather
 * than with the two surfaces in this app that happened to call the same
 * function.
 *
 * Which KIND of attention it is did not go with it: `Terminal.activityLabel`
 * says "Needs you", "Done" or "Failed" in words, and `rowStatus` puts one of
 * them on every agent row. Recorded here rather than left implicit, because
 * "the failure red is gone" reads like a regression until you know where the
 * word went.
 */

/**
 * `+82 -13`: how much a worktree has changed, in the two colors those signs
 * have everywhere in this app.
 *
 * One copy, not one per surface. The front door's review row wrote this out and
 * the worktree's Changes chip was about to write it out again, which is the
 * shape `df87410` already had to pull the landing ordering back from: three
 * copies of one rule is three chances for a phone to disagree with itself.
 *
 * The green is a literal because it is the one color Material's scheme has no
 * role for — there is no "positive" slot in a `ColorScheme` the way there is an
 * `error` one. It used to match `attentionColor`'s green to the byte and was
 * written out separately so that a change to what a FINISHED AGENT looks like
 * could not silently change what an added line looks like — and that separation
 * has now been cashed in twice. A finished agent wears `GlancePalette.review`,
 * the low-chroma review ink, and no green at all; `FINISHED` green survives only
 * on the things INSIDE a turn, a plan step and a tool call. Neither move touched
 * this constant. Red IS the scheme's error role, because "removed" and "went
 * wrong" want the same red under every theme.
 *
 * **This is NOT a glance color and must not become one.** §01's palette is
 * about what an agent is doing; a diff's signs are about what a patch says, and
 * they sit inside a review surface a person is already reading rather than on a
 * mark they are meant to catch at a glance.
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

/**
 * The green a diff's insertions are drawn in. See [DiffCounts].
 *
 * Internal rather than private since the review surface landed: `ChangesScreen`
 * draws the same pair on a file heading, on a commit's counts and on every added
 * line of a patch, and a fifth hand-mixed green three files away is precisely
 * what [DiffCounts]'s own comment says one copy exists to prevent.
 */
internal val DIFF_ADDED = Color(0xFF4CAF50)

/**
 * How much of an ink a full-width wash gets, where a row is TINTED rather than
 * drawn in a color.
 *
 * The added and removed backgrounds behind a patch's lines. It was `0x26` baked
 * into two ARGB literals — 38/255, which is this — and written as part of the
 * color rather than as a property of the wash, so the two could not be changed
 * together and neither could follow a theme's own error red.
 */
internal const val WASH = 0.15f


/**
 * The system bars' icons, in the app's resolved mode (ov-690).
 *
 * `enableEdgeToEdge()` picks light or dark icons from the PHONE's mode, once,
 * when it is called; this activity handles `uiMode` itself, so it is not called
 * again when the phone flips. With Appearance on Light over a dark phone that
 * left white clock and battery icons on a white screen. Called again whenever
 * the app's mode changes, with the same transparent bars and nav scrims the
 * default draws.
 */
@Composable
fun SystemBarsFollowTheApp() {
    val activity = LocalContext.current as? androidx.activity.ComponentActivity ?: return
    val dark = LocalGlanceDark.current
    SideEffect {
        val transparent = android.graphics.Color.TRANSPARENT
        activity.enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.auto(transparent, transparent) { dark },
            navigationBarStyle = SystemBarStyle.auto(
                android.graphics.Color.argb(0xe6, 0xFF, 0xFF, 0xFF),
                android.graphics.Color.argb(0x80, 0x1b, 0x1b, 0x1b),
            ) { dark },
        )
    }
}
