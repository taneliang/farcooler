package com.farcooler.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.material3.MaterialTheme
import com.farcooler.model.AgentActivity
import com.farcooler.model.AgentOutcome
import com.farcooler.model.GlanceInk
import com.farcooler.model.GlanceMark
import com.farcooler.model.GlanceMarkSize
import com.farcooler.model.GlancePalette
import com.farcooler.model.GlanceType
import com.farcooler.model.Terminal
import com.farcooler.model.agentOutcome

/*
 * The glance vocabulary, drawn.
 *
 * The Compose half of `model/Glance.kt`, and deliberately the only half that
 * knows what a `Color` is. Everything decidable — which ink, which stroke,
 * which words — is decided over there where a JVM test can read it; this file
 * turns those answers into pixels and does not add to them.
 *
 * That split is the same one `model/NeedsYou.kt` and `ui/PaneDeck.kt` already
 * make, and it exists here for a sharper reason than testability: a colour
 * system's bugs are almost all in the arithmetic, and arithmetic that lives
 * inside a composable can only be checked by looking at it.
 */

/**
 * Which of the two palettes this subtree is drawing in.
 *
 * **The app's own theme, not the system's.** iOS's port asks
 * `@Environment(\.colorScheme)`; `FarCoolerTheme` states at length why this app
 * does not ask `isSystemInDarkTheme()` — half of it is a terminal, the terminal's
 * palette is a preference the person set, and chrome that went the other way
 * would read as two applications. So the glance surfaces follow the same
 * decision every other surface in the app follows.
 *
 * `staticCompositionLocalOf` rather than `compositionLocalOf`: this changes when
 * somebody picks a theme in settings, which already recomposes the whole tree
 * through `Themes.revision`, and the static form costs nothing to read.
 *
 * Defaults to dark for a composable drawn outside [FarCoolerTheme] — a preview,
 * a test — because the spec's dark column is the half it fully specifies.
 */
val LocalGlanceDark: ProvidableCompositionLocal<Boolean> = staticCompositionLocalOf { true }

/** Provide [LocalGlanceDark]. Called by `FarCoolerTheme` and by nothing else. */
@Composable
fun ProvideGlanceAppearance(dark: Boolean, content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalGlanceDark provides dark, content = content)
}

/** One of the twelve, resolved for the appearance this subtree is in. */
@Composable
@ReadOnlyComposable
fun glanceColor(ink: GlanceInk): Color = Color(ink.argb(LocalGlanceDark.current))

/**
 * The ink a mark's core is filled with, and anything else read first.
 *
 * **§01 gives `text 1` and `text 2` one figure each, and both are light inks for
 * a dark surface.** Every light-mode value the spec DOES state is an inversion
 * or a darkening — surfaces go to translucent black, amber darkens, review
 * darkens, "Trace tones invert" — so the neutrals plainly invert too. It just
 * does not say to what, and §09 already knows this section is unfinished:
 * "Light mode is specified as values but not drawn. Worth one pass before
 * build."
 *
 * Drawing the mark in light mode is what turned that from a note into a defect:
 * `text 1` is L 0.97, so a present core on a pale surface is a near-white disc
 * on a near-white ground, and "producing" and "at a prompt" become the same
 * drawing.
 *
 * **So light mode defers to the theme's own on-surface roles rather than
 * inventing two numbers.** `onSurface` and `onSurfaceVariant` are correct on any
 * appearance by construction, and they are what the rest of this app's chrome
 * already uses. Dark mode keeps the spec's figures exactly, which is the half
 * the spec actually specifies. When the design document draws light mode, these
 * two arms become literals like every other value.
 *
 * This is the same deferral iOS's port makes to `Color.primary` / `.secondary`,
 * arrived at for the same reason and stated in the same place — see
 * `GlancePalette.ink1`.
 */
@Composable
@ReadOnlyComposable
fun glanceInk1(): Color =
    if (LocalGlanceDark.current) Color(GlancePalette.text1.darkArgb)
    else MaterialTheme.colorScheme.onSurface

/** The secondary ink, on the same argument as [glanceInk1]. The quiet hairline. */
@Composable
@ReadOnlyComposable
fun glanceInk2(): Color =
    if (LocalGlanceDark.current) Color(GlancePalette.text2.darkArgb)
    else MaterialTheme.colorScheme.onSurfaceVariant

/**
 * The mark, drawn. One composable for every surface in the product.
 *
 * Where a caller has a `Terminal` or an `InboxRow` it builds the mark from that
 * — `GlanceMark.of`, `GlanceMark.ofDiff` — rather than deciding anything
 * itself, so two screens showing the same fleet cannot draw it two ways.
 *
 * ## Why a `Canvas` and not three nested `Box`es
 *
 * A border is drawn INSIDE the shape here, the way iOS's `strokeBorder` is and
 * unlike `Modifier.border`, which straddles the path: a 15dp lone indicator with
 * a 3.5dp ring would come out 18.5dp across, and two surfaces sized from the
 * same table would quietly stop lining up. The inset is one subtraction in
 * [drawMark], via [GlanceMarkSize.ringRadius] — which is a figure in
 * `model/Glance.kt`, like every other figure in this vocabulary, so that a test
 * can assert the outer edge lands where §03 says it does. [GlanceMark.dash]
 * lives there for the same reason.
 *
 * @param decorative whether this mark is the only thing saying what it says.
 *   A ribbon of eight marks beside eight labelled rows is decoration and should
 *   be silent to TalkBack; a lone indicator that replaces a word is not, and
 *   announces [GlanceMark.phrase]. **This is the whole accessibility story for
 *   the mark**: it is a difference in line width, fill and dash, none of which a
 *   screen reader can perceive, so a mark that is neither labelled nor beside a
 *   label is a mark that does not exist for a TalkBack user.
 */
@Composable
fun GlanceMarkView(
    mark: GlanceMark,
    size: GlanceMarkSize,
    modifier: Modifier = Modifier,
    decorative: Boolean = false,
) {
    val ring = when (mark.attention) {
        GlanceMark.Attention.NEEDS_YOU -> glanceColor(GlancePalette.amber)
        GlanceMark.Attention.TO_REVIEW -> glanceColor(GlancePalette.review)
        // Never a hue. §03 gives the quiet tier a hairline and nothing else,
        // and the point of that is subtractive: a list of twenty rows where
        // nineteen are quiet has to have nineteen marks that do not compete
        // with the one that is not.
        GlanceMark.Attention.QUIET -> glanceInk2()
    }
    // Never amber. §03 reserves the ring for the person's side and the core for
    // the agent's, and amber is the person's colour: an amber core would say
    // "this needs you" about the half of the mark that is only ever saying what
    // the agent is doing.
    val core = glanceInk1()

    Canvas(
        modifier
            .size(size.diameter.dp)
            .then(
                if (decorative) Modifier.clearAndSetSemantics {}
                else Modifier.semantics { contentDescription = mark.phrase }
            )
    ) {
        drawMark(mark, size, ring, core)
    }
}

/**
 * The tint for a state the glance vocabulary has a mark for, or an outcome it
 * does not — and null where there is nothing to say.
 *
 * **This is `Status.tint` from the Mac**, and it is one function for the same
 * reason it is one there: the glyph was not the only thing painting a status.
 * A tab-strip chip's ring and the fleet row it names must not be able to come
 * out different colours, and before the Mac fused this, a failed agent was
 * orange collapsed and red expanded.
 *
 * **Amber comes from [GlancePalette] and is a function of the appearance,
 * because §01 says light mode is "Not a filter flip" — amber darkens to hold
 * contrast on a pale backdrop. Green and red do not come from the palette,
 * because §01 has no figure for either**: they are this app's own inks for the
 * two states §03 does not cover, and [agentOutcome] is where that is argued.
 *
 * Red is `colorScheme.error`, which is the same choice `DiffCounts` makes and
 * for the same reason — "removed" and "went wrong" want the same red under
 * every theme, and Material has a role for it. Green is a literal because
 * Material has no "positive" role at all.
 */
@Composable
fun agentTint(terminal: Terminal): Color? = when {
    agentOutcome(terminal) == AgentOutcome.FAILED -> MaterialTheme.colorScheme.error
    agentOutcome(terminal) == AgentOutcome.DONE -> FINISHED
    terminal.agent == AgentActivity.BLOCKED -> glanceColor(GlancePalette.amber)
    else -> null
}

/**
 * The green for **something that ran and succeeded** — a turn that finished, a
 * plan step ticked off, a tool call that completed.
 *
 * Material green 500, which is what `attentionColor` spent on DONE before any of
 * this and what the Mac's `.green` is the nearest system equivalent of. §01 has
 * no figure for it, and that is not an oversight to be filled in from the
 * palette: the glance vocabulary is about whether an agent wants you and whether
 * it is producing, and "it worked" is neither. [agentOutcome] is where that gap
 * is argued.
 *
 * **One meaning, one value, three call sites.** The same literal was written out
 * four times in `ui/AgentRows.kt` and here, for a finished turn, a done plan
 * step and a completed tool call — which are one fact at three scales — plus a
 * fifth time for a diff's insertions, which is NOT. So the first four collapse
 * here and the fifth stays where it is.
 *
 * **`DIFF_ADDED` is deliberately still a separate literal of the same bytes.**
 * `DiffCounts` makes the argument and it survives the vocabulary intact: a `+`
 * on a patch is about what a change SAYS, not about how something turned out,
 * and a decision to restyle "finished" must not silently restyle every added
 * line in a review.
 */
internal val FINISHED = Color(0xFF4CAF50)

/**
 * One agent's state, drawn: §03's mark where there is one, and the outcome
 * drawing where there is not.
 *
 * **The Android `StatusGlyph`.** Every surface that shows what an agent is doing
 * calls this rather than deciding for itself, so a fleet row and a tab chip
 * cannot draw the same terminal two ways — which is the disagreement the Mac had
 * to be pulled back from, and which this app had its own version of when the row
 * drew a tinted icon and the chip drew a ring.
 *
 * The box is claimed at [size]'s diameter whether or not anything occupies it,
 * so names align down a list rather than stepping in and out as agents finish.
 *
 * @param decorative whether the row already says this in words. **This is the
 *   whole accessibility story**: the mark is a difference in line width, fill
 *   and dash, none of which TalkBack can perceive. Where it is not decorative it
 *   announces [Terminal.activityLabel] — the app's own word, "Needs you",
 *   "Working", "Done", "Failed" — and NOT [GlanceMark.phrase]. That is the Mac's
 *   choice and its reason is sharp: the states §03 has no slot for are precisely
 *   the ones whose whole content is the word, and a screen reader that heard
 *   "Nothing wanted" for a turn that died would be told the opposite of the
 *   truth.
 */
@Composable
fun AgentMarkView(
    terminal: Terminal,
    now: Long,
    size: GlanceMarkSize,
    modifier: Modifier = Modifier,
    decorative: Boolean = false,
) {
    val label = terminal.activityLabel
    val tint = agentTint(terminal)
    Box(
        modifier
            .size(size.diameter.dp)
            .then(
                if (decorative) Modifier.clearAndSetSemantics {}
                else Modifier.semantics { contentDescription = label }
            ),
        contentAlignment = Alignment.Center,
    ) {
        val mark = GlanceMark.of(terminal, now)
        if (mark != null) {
            // Decorative unconditionally: this composable has already said the
            // word above, and a mark that announced `phrase` too would make
            // TalkBack read two descriptions of one dot.
            GlanceMarkView(mark, size, decorative = true)
        } else if (tint != null) {
            // Filled, at the mark's own diameter. Filled rather than hollow
            // because both outcomes are definite — hollow is this app's shape
            // for a missing answer, and `ProcessDot` owns it.
            Canvas(Modifier.size(size.diameter.dp)) {
                drawCircle(color = tint, radius = this.size.minDimension / 2)
            }
        }
        // Neither, which is a pane with no agent in it: the box stays claimed
        // and nothing is drawn. `ProcessDot` is already saying whether the
        // process is alive, and this column is not about that.
    }
}

/**
 * The drawing itself, in one place for every size and every state.
 *
 * Separate from the composable so it can be called from a `Canvas` that is
 * already drawing something else — a row that paints its own background, an
 * overview card — without that caller reimplementing the inset or the dash.
 */
fun DrawScope.drawMark(
    mark: GlanceMark,
    size: GlanceMarkSize,
    ring: Color,
    core: Color,
) {
    val stroke = size.stroke(mark.attention).dp.toPx()
    val centre = Offset(this.size.width / 2, this.size.height / 2)

    drawCircle(
        color = ring,
        // Inset by half the stroke, so the mark's OUTER diameter is exactly the
        // diameter §03 names rather than the diameter plus a ring. The
        // subtraction is [GlanceMarkSize.ringRadius] and not an expression here,
        // because it is a figure and every figure in this vocabulary lives in
        // `model/Glance.kt` where a test can read it.
        radius = size.ringRadius(mark.attention).dp.toPx(),
        center = centre,
        style = Stroke(
            width = stroke,
            pathEffect =
                if (mark.link == GlanceMark.Link.BROKEN)
                    PathEffect.dashPathEffect(GlanceMark.dash(stroke), 0f)
                else null,
        ),
    )

    if (mark.core == GlanceMark.Core.PRODUCING) {
        val coreDiameter = size.core ?: return
        drawCircle(color = core, radius = coreDiameter.dp.toPx() / 2, center = centre)
    }
}

/**
 * One row of §02's type table, as a `TextStyle`.
 *
 * Size and tracking travel together because that is how the table is written and
 * because two of the three are meaningless alone — 17/600 with no tracking is a
 * different style from the one specified, and nothing on screen would say so.
 * Compose puts them on the same object, which is the one thing this port has
 * easier than SwiftUI, where they are two modifiers and the second is easy to
 * forget.
 *
 * **Tabular figures, always.** §02 makes it universal, and it is not a
 * preference: these surfaces re-render on a poll, and a column of proportional
 * numerals jitters sideways between refreshes, which reads as the layout being
 * unstable rather than as the number having changed. `TextGeometricTransform` is
 * not what does that — Compose reaches tabular figures through a font feature
 * setting, which is what [fontFeatureSettings] carries here.
 */
@Composable
@ReadOnlyComposable
fun glanceTextStyle(style: GlanceType): TextStyle = TextStyle(
    fontSize = style.size.sp,
    fontWeight = FontWeight(style.weight),
    letterSpacing = style.tracking.sp,
    fontFamily = if (style.mono) FontFamily.Monospace else FontFamily.Default,
    fontFeatureSettings = "tnum",
)
