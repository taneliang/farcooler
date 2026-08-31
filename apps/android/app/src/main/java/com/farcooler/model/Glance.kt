package com.farcooler.model

import kotlin.math.cos
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.roundToInt
import kotlin.math.sin

/*
 * The glance vocabulary: one mark, twelve colours, six type styles.
 *
 * A port of `apps/shared/AgentKit/Sources/AgentKit/GlanceMark.swift`,
 * `GlancePalette.swift` and `GlanceType.swift`, which are themselves a
 * transcription of the design project's `Spec.dc.html`. That document is
 * authoritative for every literal in this file and says so about itself:
 *
 * > Do not copy values out of them into your own constants file and then edit
 * > them there — that duplication is what produced eight rounds of drift while
 * > these documents were being written, and the same failure mode will hit the
 * > codebase.
 *
 * So: a surface that wants amber asks [GlancePalette.amber]. It does not write
 * `Color(0xFFFF9800)`, and it does not write `oklch(0.78 0.16 70)` a second
 * time. Four copies of a hand-mixed green and one hand-mixed orange existed in
 * this app when this file was written — `attentionColor`, `DIFF_ADDED`, and
 * three more in `ui/AgentRows.kt` — which is exactly the shape the rule above
 * describes.
 *
 * ## Why this is in `model/` and holds no Compose type
 *
 * Everything here is arithmetic on `Double` and plain data, so it runs under
 * `testInstrumentedUnitTest` on the JVM with no device and no Robolectric. The
 * conversion from OKLCH to sRGB is sixteen constants and a transfer function;
 * it is the one part of a colour system that can be WRONG rather than merely
 * ugly, and a wrong matrix is invisible on a screenshot and obvious in a test.
 *
 * `ui/Glance.kt` is the thin Compose layer that turns an [Int] ARGB into a
 * `Color` and a [GlanceMark] into a drawing. Nothing here imports Compose,
 * which is also why sizes are `Float` points rather than `Dp`: this file states
 * the spec's figures, and the `.dp` that makes them a layout belongs at the
 * draw site.
 *
 * ## Where this deliberately differs from iOS
 *
 * iOS resolves light-versus-dark from `@Environment(\.colorScheme)`, which is
 * the system appearance. This app's chrome follows the chosen TERMINAL theme
 * instead — see `ui/Theme.kt`, whose whole argument is that a light list
 * handing off to a black terminal reads as two applications — so [GlanceInk]
 * takes a plain `dark: Boolean` and the Compose layer feeds it
 * `Themes.current.dark`. Same two literals, a different question asked of the
 * device.
 */

/**
 * One colour, in the co-ordinates the spec uses.
 *
 * OKLCH is the stored form rather than a hand-converted hex table, and that is
 * the whole point of this type. The spec's §09 review pass records what
 * happened when its own swatch table printed bare numbers without their
 * function — "A spec exists to be copied; all twelve are now complete `oklch()`
 * strings" — and a table of pre-converted hex here would reintroduce exactly
 * that gap: nobody comparing this file against the design document could tell
 * whether `0xFFF7A224` was still `oklch(0.78 0.16 70)` or had been nudged.
 *
 * [lightness] is 0…1, [chroma] is absolute (roughly 0…0.4 for anything a screen
 * can show), [hue] is degrees, [alpha] is 0…1.
 */
data class Oklch(
    val lightness: Double,
    val chroma: Double,
    val hue: Double,
    val alpha: Double = 1.0,
) {
    /**
     * The sRGB colour, as a packed ARGB `Int`, converted here rather than at any
     * call site.
     *
     * Björn Ottosson's Oklab matrices, in the order the transform runs: polar →
     * Oklab, Oklab → cone responses, cubed, cone responses → linear sRGB, then
     * the sRGB transfer function. Written out rather than pulled from a
     * dependency because sixteen constants that never change are cheaper to
     * read — and cheaper to test — than a library's version.
     *
     * **Out-of-gamut components are clamped, not gamut-mapped**, which is a
     * deliberate simplification and safe for exactly these values: every colour
     * in [GlancePalette] was checked against Chrome's own CSS Color 4
     * implementation and lands inside sRGB, so no clamp fires today. It is here
     * so that a value edited in the design document to something sRGB cannot
     * hold degrades to the nearest displayable colour rather than to whatever a
     * negative component rounds to.
     */
    fun toArgb(): Int {
        val radians = hue * Math.PI / 180.0
        val a = chroma * cos(radians)
        val b = chroma * sin(radians)

        val lRoot = lightness + 0.3963377774 * a + 0.2158037573 * b
        val mRoot = lightness - 0.1055613458 * a - 0.0638541728 * b
        val sRoot = lightness - 0.0894841775 * a - 1.2914855480 * b

        val l = lRoot * lRoot * lRoot
        val m = mRoot * mRoot * mRoot
        val s = sRoot * sRoot * sRoot

        val red = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
        val green = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
        val blue = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s

        return (channel(alpha) shl 24) or
            (encode(red) shl 16) or
            (encode(green) shl 8) or
            encode(blue)
    }

    /** Linear light to an sRGB byte, clamped into the unit interval first. */
    private fun encode(linear: Double): Int {
        val c = min(1.0, max(0.0, linear))
        return channel(if (c <= 0.0031308) 12.92 * c else 1.055 * c.pow(1 / 2.4) - 0.055)
    }

    private fun channel(unit: Double): Int = (unit * 255.0).roundToInt().coerceIn(0, 255)
}

/**
 * One colour of the system, in both appearances.
 *
 * Two literals rather than one colour and a filter, because §01 is explicit
 * that light mode is "Not a filter flip": amber DARKENS on a pale backdrop to
 * hold its contrast, the surfaces invert to translucent black, and the two
 * trace tones swap ends of the scale. A single value adjusted at draw time
 * cannot express any of that.
 */
data class GlanceInk(val dark: Oklch, val light: Oklch) {
    /** Same value in both appearances — the neutrals §01 gives one figure for. */
    constructor(both: Oklch) : this(both, both)

    /** The packed ARGB for this appearance. */
    fun argb(dark: Boolean): Int = (if (dark) this.dark else light).toArgb()

    /**
     * The dark value, for surfaces that are dark whatever the app is set to — a
     * notification or a widget sits over a wallpaper and takes no part in the
     * app's own theme choice.
     */
    val darkArgb: Int get() = dark.toArgb()
}

/**
 * §01, transcribed. Twelve values, and nothing else in the product may hold a
 * colour that belongs to a glance surface.
 */
object GlancePalette {
    // The one saturated hue, and the one that is not allowed to be loud.

    /**
     * Needs you. **Nothing else in the product may be amber, at any opacity.**
     *
     * This is the whole colour system: one reserved hue, so that "does this need
     * me" is answered before a word is read. It replaces `attentionColor`'s
     * `Color(0xFFFF9800)` — Material orange 500, which was one step from the
     * Material amber 500 that `ui/Theme.kt` had already argued had to be kept
     * off the process dot for being indistinguishable from it.
     *
     * The light value is a genuinely different colour rather than the same one
     * dimmed — §01: "amber darkens to oklch(0.62 0.13 68) to hold contrast on a
     * pale backdrop."
     */
    val amber = GlanceInk(dark = Oklch(0.78, 0.16, 70.0), light = Oklch(0.62, 0.13, 68.0))

    /**
     * A finished diff, unread. Deliberately low chroma so amber stays the only
     * loud thing.
     *
     * **A finished diff nobody has read, and a finished turn nobody has
     * opened.** The tier was the first of those alone until `done` was let into
     * it on all three platforms; what it says now is "this is over, and you have
     * not looked".
     *
     * **The workspace counts still have no per-agent version.** An `InboxRow` is
     * a WORKSPACE's counts and `model/NeedsYou.kt` refuses to invent a per-agent
     * one; that refusal stands, because the ban's reason was invented data.
     * `done` was never invented — the daemon sends it per terminal — which is
     * why it is the one thing the narrowing let in.
     *
     * [GlanceMark.of] produces this tier for `done` and for nothing else, and
     * [ofDiff] for an unread diff. Those two, and no third.
     */
    val review = GlanceInk(dark = Oklch(0.76, 0.055, 235.0), light = Oklch(0.5, 0.06, 235.0))

    // The activity trace's four tones.
    //
    // Kept in full although nothing draws a trace yet — the data for it does
    // not exist at any layer, which is a daemon question rather than a view
    // one. §09 argues for keeping all twelve rows even though a strict reading
    // would cut the table to the four that vary: "a value absent from a spec
    // gets invented at build time, and inventing an amber is the one mistake
    // that breaks the whole system."

    /** Upper half of the trace — lines touched. Inverts in light mode. */
    val code = GlanceInk(dark = Oklch(0.88, 0.002, 250.0), light = Oklch(0.28, 0.002, 250.0))

    /** Lower half of the trace — output to the person. Inverts in light mode. */
    val chat = GlanceInk(dark = Oklch(0.62, 0.002, 250.0), light = Oklch(0.5, 0.002, 250.0))

    /** Commit marks on the trace axis. */
    val commit = GlanceInk(Oklch(0.96, 0.002, 250.0))

    /** The trace centre rule. Continuous, never dotted. */
    val axis = GlanceInk(Oklch(0.44, 0.002, 250.0))

    /** A bucket with no activity. Drawn, not omitted. */
    val empty = GlanceInk(Oklch(0.42, 0.002, 250.0))

    // Ink.

    /** Names, counts, anything you read first. */
    val text1 = GlanceInk(Oklch(0.97, 0.002, 250.0))

    /**
     * Secondary lines inside a frame. **Floor for on-device text** — §01's
     * contrast rule is that nothing below L 0.7 goes on the card, and this is
     * L 0.74.
     */
    val text2 = GlanceInk(Oklch(0.74, 0.004, 250.0))

    // Surfaces. §01 gives one figure per surface for dark and a RANGE for light
    // — "Surfaces invert to black at 8% → 3%" — over the three surfaces in the
    // order the table lists them. The two ends are the spec's; the widget's
    // 5.5% is the midpoint of a two-point range read across three rows, and is
    // the one number here that is derived rather than quoted. The iOS port
    // carries the same note and the same figure, so the two platforms are
    // wrong in the same place if it is wrong at all.

    /** A card over a lock screen. */
    val card = GlanceInk(
        dark = Oklch(0.22, 0.004, 250.0, alpha = 0.9),
        light = Oklch(0.0, 0.0, 0.0, alpha = 0.08),
    )

    /** A home-screen widget's surface. */
    val widget = GlanceInk(
        dark = Oklch(0.19, 0.004, 250.0, alpha = 0.95),
        light = Oklch(0.0, 0.0, 0.0, alpha = 0.055),
    )

    /** The densest surface in the system. */
    val island = GlanceInk(
        dark = Oklch(0.12, 0.002, 250.0),
        light = Oklch(0.0, 0.0, 0.0, alpha = 0.03),
    )
}

/**
 * What one mark says, on all three of its axes.
 *
 * **Ring is your side, core is the agent's.** That sentence is the whole of §03
 * of the design spec:
 *
 *  - The RING's weight and hue are the person's side — is your attention
 *    wanted, and what for. Three weights: needs-you, to-review, hairline.
 *  - The CORE is the agent's — filled while producing, absent at a prompt.
 *  - DASHES go on the ring, because the ring IS the channel. A broken ring is a
 *    broken link, and it never disturbs the core.
 *
 * Three rules, twelve states, "so a combination you have not met still reads
 * correctly". That last clause is why this is three independent axes rather
 * than an enum of twelve cases: an enum has to have every combination written
 * down before it can be drawn, and the one nobody wrote down is the one that
 * turns up on a phone.
 *
 * **What this replaces on Android.** Four surfaces drew four different things
 * and shared nothing but a hue: the fleet row drew one of six Material icons
 * tinted orange-or-green, the tab strip drew a 1.5dp ring around a chip and
 * flipped the label's font weight, `ProcessDot` drew an eighth-inch disc about
 * a different fact entirely, and `AgentRows` drew a second, smaller dot
 * vocabulary of its own. A person seeing two of those within a minute had no
 * way to tell they were about the same thing.
 */
data class GlanceMark(
    val attention: Attention,
    /**
     * What the agent is doing, or null where this surface may not say.
     *
     * **Null is what a widget uses, and it is not a thirteenth state.** §08 is
     * explicit: "Working versus idle never appears on a widget. It flips every
     * few seconds; at this refresh rate the claim would be false more often
     * than true."
     *
     * So the twelve states are still the drawn vocabulary; null is a SURFACE
     * declining to state one of the three axes, which is a different thing from
     * stating that the agent is at a prompt. It draws the same as [Core.AT_A_PROMPT]
     * — there is no fourth thing a core can look like — but it says nothing in
     * [phrase], where the difference is audible.
     */
    val core: Core?,
    val link: Link = Link.LIVE,
) {
    /** The ring: whether your attention is wanted, and what for. */
    enum class Attention {
        /**
         * An agent is stopped, waiting on a person. The heaviest ring, and the
         * only amber thing in the product.
         */
        NEEDS_YOU,

        /**
         * A finished thing nobody has looked at — a diff nobody has read, or a
         * turn that ended and nobody has opened. A middle-weight ring in the low
         * chroma [GlancePalette.review].
         *
         * **Still never a per-agent `reviewsWaiting` or unread-diff count** —
         * those are a WORKSPACE's, and `model/NeedsYou.kt` refuses to invent a
         * per-agent version of either. `done` is the one agent state that
         * reaches this tier. See [GlancePalette.review].
         */
        TO_REVIEW,

        /** Nothing is wanted from you. A 1dp hairline at every size. */
        QUIET,
    }

    /**
     * The core: what the agent itself is doing.
     *
     * Two values and no third, because §03 gives the core exactly one job. "Not
     * stated" is `null` on [core] rather than a case here.
     */
    enum class Core {
        /** Producing. A filled disc at the centre. */
        PRODUCING,

        /** At a prompt. No disc. */
        AT_A_PROMPT,
    }

    /** The ring's continuity: whether the channel is currently carrying anything. */
    enum class Link {
        LIVE,

        /**
         * Unreachable, or a claim about the present that has gone stale. Drawn
         * as a dashed ring.
         *
         * One axis for both, because they are one fact: §08's staleness rule is
         * that "decay applies only to claims about the present. Blocked and
         * to-review hold at any age; working and idle go dashed" — which is the
         * same statement as a broken link, made about time instead of about the
         * network.
         */
        BROKEN,
    }

    /**
     * Whether this mark is one of the ones a vibrancy fallback drops.
     *
     * §03's stated concession: "If 1pt hairlines wash out under vibrancy, quiet
     * marks leave the accessory ribbon and only the attention marks are drawn;
     * the count text carries the rest."
     */
    val isQuiet: Boolean get() = attention == Attention.QUIET

    /** The same mark with the core withheld — what every widget family draws. */
    val withoutCore: GlanceMark get() = copy(core = null)

    /**
     * The tier, in words, for TalkBack.
     *
     * **Stroke weight is not exposed to a screen reader, and neither is hue.**
     * The entire mark is a difference in line width, fill and dash — which is to
     * say the entire mark is invisible to TalkBack unless it is also said out
     * loud. The words are the matrix's own row and column labels from §03, so
     * what is spoken and what is drawn are the same table.
     */
    val phrase: String
        get() {
            val tier = when (attention) {
                Attention.NEEDS_YOU -> "Needs you"
                Attention.TO_REVIEW -> "To review"
                Attention.QUIET -> "Nothing wanted"
            }
            if (link == Link.BROKEN) return "$tier, unreachable"
            return when (core) {
                Core.PRODUCING -> "$tier, producing"
                Core.AT_A_PROMPT -> "$tier, at a prompt"
                // The surface declined to say. Saying "at a prompt" here would
                // be this file inventing the very claim `core == null` exists to
                // withhold.
                null -> tier
            }
        }

    companion object {
        /**
         * How long a claim about the present may go unrefreshed before the ring
         * goes dashed.
         *
         * An hour, matching `FleetSnapshot.staleAfter` and `ShellScreen`'s
         * `staleAfter` on iOS to the second, because a phone and a watch looked
         * at within a minute of each other must not disagree about which agents
         * are still being vouched for.
         *
         * **This does not violate "the phone never computes a terminal's
         * state".** It is the age of the DAEMON's own answer, rendered:
         * [Terminal.activitySince] is a host-supplied timestamp, and a threshold
         * on it says how long ago the runner last told us anything — a fact
         * about our knowledge, which cannot disagree with the daemon about what
         * the terminal is doing. It is drawn UNDER whatever the daemon's state
         * already says, never replacing it.
         */
        const val STALE_AFTER_MS = 60L * 60L * 1000L

        /**
         * The dash a broken link is drawn with: `[on, off]`, in whatever unit
         * the stroke was given in.
         *
         * **§03's rule here cannot be implemented as written, and this is the
         * nearest thing that draws.** The spec says: "Dash pattern is the
         * platform default; only the stroke changes." Neither SwiftUI nor
         * Compose HAS a platform default — both treat an absent pattern as a
         * solid line — so a figure had to be chosen, and iOS chose this one
         * first for a reason worth repeating: a single constant array cannot
         * serve both a 1dp hairline and a 3.5dp lone indicator. At the heavy
         * end, a fixed 2-on-2-off segment is a bar two units long and three and
         * a half wide, and the mark turns into a sunburst — which is to say
         * into a spinner, which is to say the drawing for "unreachable" reads
         * as "working".
         *
         * So: one RULE rather than one array — the segment is half again as
         * long as the stroke is wide, and the gap matches the stroke. That
         * keeps the half of the spec's sentence that is a design decision
         * (there is ONE dash in the system, and states differ by stroke weight
         * and nothing else) and gives up the half that is an implementation
         * detail of a platform that does not have it.
         *
         * **This is not the "never a percentage" rule being broken.** That rule
         * is about stroke WIDTH, and its reason is grid alignment: "a ratio
         * cannot land on the half-pixel grid at these sizes." A dash runs ALONG
         * the path, around a curve, where there is no pixel grid to land on.
         *
         * It remains the one figure in this vocabulary not quoted from the
         * design document, and the document is where it should be settled — for
         * both platforms at once, since they now agree.
         */
        fun dash(stroke: Float): FloatArray = floatArrayOf(stroke * 1.5f, stroke)

        /**
         * One agent's mark, from the wire — **or null where §03 has no slot for
         * what this terminal is.**
         *
         * **Null is a refusal, not an omission**, and it is the same refusal
         * `Status.glanceMark` makes on the Mac, arrived at there first and
         * copied here deliberately. The glance vocabulary is three axes: a ring
         * that says whether YOUR attention is wanted and what for, a core that
         * says whether the AGENT is producing, and a dash that says the channel
         * is broken. `blocked`, `working`, `idle`, a FINISHED turn and a pane
         * with no agent are one of those combinations exactly, and they are
         * mapped below. A turn that DIED is not, and the nearest mark for it is
         * the quiet hairline — which is the mark for "nothing is wanted from
         * you", spoken by TalkBack as exactly that. Drawing that on a build that
         * failed overnight would make this app state the opposite of what it
         * knows, in a vocabulary that is now trusted on three platforms. So a
         * dead turn, and only a dead turn, is left to [agentOutcome].
         *
         * **[Attention.TO_REVIEW] is reachable from here, and `done` is the one
         * status that reaches it.** The tier used to forbid an agent's state in
         * so many words, on the stated ground that the review counts are per
         * WORKSPACE and a per-agent version would have to be invented. That
         * ground never reached `done` — nothing is invented, the daemon sends it
         * per terminal — so the prohibition was narrowed to the two counts it
         * was written about. `Status.glanceMark` on the Mac and
         * `GlanceMark(agent:)` on the phone make the identical mapping, which is
         * the point: one agent, one tier, three platforms. The ban still stands
         * for the workspace counts, and [ofDiff] is still the only other way to
         * this tier. See [GlancePalette.review].
         *
         * **This is not the reduction an earlier version of this file made.**
         * It folded `done` and a failed turn into the AMBER ring on the grounds
         * that `wantsAttention` covers both and that `rowStatus` prints the word
         * beside the mark. The first half is true and the second half is true of
         * a FLEET ROW and false of a tab-strip chip, which has no adjacent state
         * text at all: on a chip the hue was carrying the whole distinction, and
         * folding it in put back the exact defect red was introduced to fix —
         * "a turn that died and a turn that worked were the same dot". A
         * finished turn is a review ring here and a dead one is still red; the
         * two have never been the same dot and are not now.
         *
         * A terminal the host has said nothing about at all — no
         * [Terminal.activitySince] — is NOT stale. Null means "not told", which
         * is a different thing from "told a long time ago" and must never be
         * rendered as it: an older daemon sends no timestamp for anything, and
         * reading that as silence would draw a whole healthy fleet as
         * unreachable.
         */
        fun of(terminal: Terminal, now: Long): GlanceMark? {
            if (agentOutcome(terminal) != null) return null

            // Blocked is the load-bearing mapping and it is identical on all
            // three platforms — `GlanceMark(agent:)`, `GlanceMark(_ shell:)`
            // and `Status.glanceMark`: a heavy amber ring with NO core, because
            // §03 gives the ring to the person's side and the core to the
            // agent's, and a blocked agent is stopped at a prompt.
            //
            // DONE is the second of those three-way agreements and the newer
            // one: the turn is over and nobody has looked at it, which is what
            // the review tier says.
            val attention = when (terminal.agent) {
                AgentActivity.BLOCKED -> Attention.NEEDS_YOU
                AgentActivity.DONE -> Attention.TO_REVIEW
                else -> Attention.QUIET
            }
            val core =
                if (terminal.agent == AgentActivity.WORKING) Core.PRODUCING else Core.AT_A_PROMPT
            // Only the claim about the present decays. Blocked is latched — an
            // agent stopped an hour ago is still stopped — and so is done: a
            // turn that ended an hour ago has still ended. So an attention mark
            // keeps its solid ring however old the answer is, which is §08 word
            // for word ("Blocked and to-review hold at any age; working and idle
            // go dashed") and is the same answer `FleetSnapshot.Confidence`
            // gives on the Apple side by vouching for both at any age.
            //
            // MILLISECONDS, both sides. [Terminal.activitySince] is Unix
            // milliseconds off the host — `crates/cli`'s `activity_since` — and
            // `now` is `System.currentTimeMillis()`, so this is a subtraction
            // and not a conversion. It is called out because it was written as
            // a conversion first, `since * 1000`, which put every timestamp
            // seventeen centuries in the future and made the difference
            // negative: no ring would ever have gone dashed, on any fleet, and
            // nothing on screen would have said so.
            val since = terminal.activitySince
            val stale = attention == Attention.QUIET &&
                since != null &&
                now - since.toLong() > STALE_AFTER_MS
            return GlanceMark(attention, core, if (stale) Link.BROKEN else Link.LIVE)
        }

        /**
         * A workspace's diff, as a mark.
         *
         * Both halves of the condition, and they are not the same fact.
         * [InboxRow.hasDiff] is true of every worktree with work on it and stays
         * true after you have read it; `changedSinceReviewed` is the daemon's
         * watermark, and it is what makes this "there is something new here"
         * rather than "there is a branch here".
         *
         * Never broken: a diff has no activity and no timestamp, so there is no
         * answer whose age could be shown.
         */
        fun ofDiff(counts: InboxRow?): GlanceMark {
            val unread = counts != null && counts.changedSinceReviewed && counts.hasDiff
            return GlanceMark(
                if (unread) Attention.TO_REVIEW else Attention.QUIET,
                Core.AT_A_PROMPT,
            )
        }
    }
}

/**
 * A definite OUTCOME: the one thing this app draws without a mark.
 *
 * The Android half of the argument `Status.glanceMark` makes on the Mac and
 * [GlanceMark.of] repeats: this is drawn by `agentTint` rather than by a mark,
 * because the alternative on offer was the quiet hairline, and a hairline is the
 * mark's way of saying "nothing is wanted from you", which about a turn that
 * failed is not a missing answer but a wrong one.
 *
 * **This used to hold two, and [DONE] has left it.** A finished turn was an
 * outcome here — a filled green disc, then a filled review-ink one — for exactly
 * as long as `GlanceMark.Attention.TO_REVIEW` refused an agent's state. That
 * prohibition was narrowed to the workspace counts it was written about, `done`
 * became the review tier on all three platforms, and a finished turn is now a
 * middle-weight hollow ring drawn by [GlanceMark.of] like any other mark. Hue
 * agreement was never the whole of it: the Mac and the phone draw a RING, and a
 * recoloured disc would have been this app agreeing about the colour and not
 * about the vocabulary.
 *
 * **So this type is now a single axis: the turn died.** It is deliberately still
 * called [AgentOutcome] rather than being renamed to say so — the name is load
 * bearing across `Shell.kt`, the fleet row and the tab strip, and renaming it is
 * a separate decision from the one this file just took.
 *
 * **One, where the Mac has four.** Its `Status` is a fused type that also
 * carries the process — `lost` and `failed` mean the terminal is gone or never
 * started. This app keeps those apart: they are `StateKind`, and `ProcessDot`
 * has drawn them hollow, in its own column, since before any of this. So the
 * agent side has exactly the one outcome below, and it is FILLED, which is the
 * same split the Mac draws: filled is a definite outcome, hollow is a missing
 * answer.
 *
 * It is drawn at the mark's own diameter so the column is reserved either way —
 * a row must not change width because an agent finished.
 */
enum class AgentOutcome {
    /**
     * The turn died.
     *
     * The daemon sends a failed turn as `done` and says which in `turnFailed`,
     * so this is not a second activity but a flag on one — see
     * [Terminal.turnDidFail]. Red exists precisely because a turn that died and
     * a turn that worked were the same dot until it did, and that is why this
     * one did NOT follow `done` into the review tier: the mark has no failure
     * axis, and the review ring says "have a look", which about a build that
     * died overnight is not the fact.
     */
    FAILED,
}

/**
 * What this terminal's agent has finished as, or null when it has not finished
 * BADLY.
 *
 * The companion to [GlanceMark.of], and the two are exhaustive and exclusive
 * over an agent pane: exactly one of them answers, always. `GlanceTest` asserts
 * that over every activity and both values of the failure flag rather than
 * leaving it to two `when`s in different files to keep agreeing.
 *
 * Null for a pane that is not an agent at all, where there is nothing to say
 * and `ProcessDot` is already saying whether the process is alive — and null for
 * a turn that merely ENDED, which [GlanceMark.of] answers for now.
 */
fun agentOutcome(terminal: Terminal): AgentOutcome? = when {
    !terminal.agent.isAgent -> null
    // The flag and not the activity. A failed turn arrives AS `done`, so this
    // is the one distinction a `when` on the activity alone cannot make — and
    // it is the whole of what is left here now that a plain `done` has a mark.
    terminal.turnDidFail -> AgentOutcome.FAILED
    else -> null
}

/**
 * Which ink an agent wears, as a decision separate from resolving it to a
 * [Color].
 *
 * **This exists so the decision can be tested.** `agentTint` is a `@Composable`
 * and this module's unit tests are plain JVM JUnit with no Compose or
 * Robolectric harness, so as long as "a finished turn is the review ink" lived
 * inside the composable it was a rule nothing could catch being reverted — and
 * it was reverted once, deliberately, to prove exactly that. It is the same move
 * `GlanceMarkSize.ringRadius` and [GlanceMark.dash] made for the same reason:
 * the figure and the rule live in the model where a test can read them, and the
 * view is left with nothing but the resolution.
 *
 * **The glance ink travels with the case.** A future edit that wanted to paint a
 * finished turn green again would have to change [REVIEW]'s ink or [agentInk]'s
 * mapping, and `GlanceTest` reads both; there is no per-state branch left in the
 * view for it to hide in.
 *
 * @property glance the §01 ink this resolves to, or null where §01 has no figure
 *   and Material's own role is used instead.
 */
enum class AgentInk(val glance: GlanceInk?) {
    /** An agent is waiting on a person. The one saturated hue in the app. */
    AMBER(GlancePalette.amber),

    /**
     * The turn ended and nobody has looked yet — the same low-chroma ink
     * `GlanceMarkView` strokes a [GlanceMark.Attention.TO_REVIEW] ring in, so a
     * tab chip's border and the ring inside it cannot come out two colours.
     */
    REVIEW(GlancePalette.review),

    /**
     * The turn died. `colorScheme.error`, which is the same choice `DiffCounts`
     * makes and for the same reason — "removed" and "went wrong" want the same
     * red under every theme, and Material has a role for it where §01 has no
     * figure at all.
     */
    ERROR(null),
}

/**
 * Which ink this terminal's agent wears, or null where it wears none.
 *
 * The order is the order it has always been in: a dead turn is checked before
 * anything else because it arrives as `done`, and a blocked agent is checked
 * last because it is the only one of the three that is a plain activity.
 */
fun agentInk(terminal: Terminal): AgentInk? = when {
    agentOutcome(terminal) == AgentOutcome.FAILED -> AgentInk.ERROR
    terminal.agent == AgentActivity.DONE -> AgentInk.REVIEW
    terminal.agent == AgentActivity.BLOCKED -> AgentInk.AMBER
    else -> null
}

/**
 * The diameters this mark is drawn at, and no others.
 *
 * §03: "Six sizes across the two bodies, no others." An enum rather than a
 * `Float` because that sentence is a rule, and a rule expressed as a parameter
 * is a rule anybody is one typo away from breaking.
 *
 * **The two wrist sizes are not here.** iOS carries them because a watchOS
 * target compiles the same file; this app has no wrist surface and no Wear
 * module, and a size no binary can draw is a size that goes stale unobserved.
 * The day a Wear module lands, `watchRow` at 14 and `watchLone` at 22 come
 * across with their own stroke triples — they are in `GlanceMarkSize.swift`
 * and in §03, which is where they should be read from rather than guessed at
 * from the four below.
 *
 * **Stroke is a literal value per diameter, never a percentage.** §03 gives the
 * reason and it is not stylistic: "a ratio cannot land on the half-pixel grid at
 * these sizes."
 */
enum class GlanceMarkSize(val diameter: Float) {
    /** 8dp, in a ribbon. */
    RIBBON(8f),

    /** 10dp, in a row. */
    ROW(10f),

    /** 11dp, in a header. */
    HEADER(11f),

    /** 15dp, as a lone indicator. */
    LONE(15f);

    /**
     * The ring's width for one attention tier, in dp, quoted.
     *
     * §03's three ladders, read down 8 / 10 / 11 / 15: needs you
     * `2 / 2.5 / 2.5 / 3.5`, to review `2 / 2 / 2 / 3`, hairline `1` at every
     * size.
     */
    fun stroke(attention: GlanceMark.Attention): Float = when (this) {
        // At 8dp the first two tiers collapse to ONE weight, and that is the
        // spec's own fallback rather than a rounding: "there is no room for four
        // distinguishable strokes, so an 8pt ribbon separates wants you from
        // quiet and leaves amber-versus-review to hue." Which is why the two
        // arms below are both 2, written out rather than merged — the day a
        // ribbon gains a point of diameter, the two numbers part company again.
        RIBBON -> when (attention) {
            GlanceMark.Attention.NEEDS_YOU -> 2f
            GlanceMark.Attention.TO_REVIEW -> 2f
            GlanceMark.Attention.QUIET -> 1f
        }
        ROW -> when (attention) {
            GlanceMark.Attention.NEEDS_YOU -> 2.5f
            GlanceMark.Attention.TO_REVIEW -> 2f
            GlanceMark.Attention.QUIET -> 1f
        }
        HEADER -> when (attention) {
            GlanceMark.Attention.NEEDS_YOU -> 2.5f
            GlanceMark.Attention.TO_REVIEW -> 2f
            GlanceMark.Attention.QUIET -> 1f
        }
        LONE -> when (attention) {
            GlanceMark.Attention.NEEDS_YOU -> 3.5f
            GlanceMark.Attention.TO_REVIEW -> 3f
            GlanceMark.Attention.QUIET -> 1f
        }
    }

    /**
     * The radius the ring's PATH is drawn on, so that the mark's outer edge
     * lands exactly on [diameter].
     *
     * **This is the difference between `strokeBorder` and `stroke`**, and it is
     * the one arithmetic mistake available in drawing this mark. A stroke
     * straddles the path it follows, so a ring centred on `diameter / 2` sticks
     * out by half its own width: a 15dp lone indicator with a 3.5dp ring comes
     * out 18.5dp across, a 10dp row mark with a 2.5dp ring comes out 12.5dp,
     * and — this is the part that is invisible until two surfaces are put side
     * by side — the overshoot is DIFFERENT per tier, so a column of marks stops
     * being a column the moment one row's agent gets blocked.
     *
     * It lives here rather than inline in the composable because it is a figure
     * derived from two of the spec's, which makes it exactly the kind of thing
     * this file exists to hold and `ui/Glance.kt` exists not to.
     */
    fun ringRadius(attention: GlanceMark.Attention): Float =
        (diameter - stroke(attention)) / 2

    /**
     * The core's diameter, or null where this size has none.
     *
     * §03, literally: "3 at 8pt, 4 at 11pt, 5 at 15pt. There is no 10pt core — a
     * row shows the ring alone — and below 7pt diameter the core comes off
     * entirely."
     *
     * The missing 10dp core is not an oversight to be filled in by
     * interpolation. A row is the densest thing in the system and the ring alone
     * is what stays legible there; splitting the difference between 3 and 4
     * would put a 3.5dp disc inside a 5dp hole and produce a mark that reads as
     * a smudge at exactly the size it is read at most.
     */
    val core: Float?
        get() = when (this) {
            RIBBON -> 3f
            ROW -> null
            HEADER -> 4f
            LONE -> 5f
        }
}

/**
 * §02's type table: six styles, one rule about which font, and nothing else.
 *
 * **The system face for chrome, monospace for anything a machine produced.**
 * §02 states it as a test rather than a list: "If it came off a machine it is
 * mono — timestamps, counts, diff stats, commit hashes, identifiers. If a
 * person wrote it, or the product did, it is SF." A headline the daemon's
 * ladder composed is still words for a person, so it is the system face; the
 * count beside it is a figure, so it is mono.
 *
 * **Numerals are always tabular.** Not a preference: these surfaces re-render on
 * a poll and a column of proportional figures jitters sideways between
 * refreshes, which reads as the layout being unstable rather than as the number
 * having changed.
 *
 * ## `sp`, where iOS uses fixed points — a deliberate divergence
 *
 * iOS's port takes §02's figures as absolute points and says so at length: §01
 * makes 11 a hard floor, and "a scaling ramp cannot honour it in both
 * directions at once". That trade is defensible on a platform where the tile is
 * a fixed size, and it is the wrong trade for this app's chrome. Android's font
 * scale reaches 200% and the setting is the platform's main accessibility
 * affordance for text; an app whose row names ignore it is an app that has
 * opted out of it. So [size] is in `sp` and the floor is honoured at the
 * default scale, which is the scale the spec's "one second at arm's length"
 * test was conducted at. A surface that genuinely cannot reflow — a widget,
 * when one exists — is where `dp` would be the right answer, and that surface
 * can say so at its own draw site.
 */
data class GlanceType(
    /** In `sp`. See the class comment for why not `dp`. */
    val size: Float,
    /** 400 or 600, in the units `FontWeight` counts in. */
    val weight: Int,
    /**
     * Letter spacing in `sp`, converted from the spec's em figure at this
     * style's own size. Compose's `letterSpacing` takes a `TextUnit`; the spec,
     * like every type spec, is written in ems because that is the unit that
     * stays true when a size changes.
     */
    val tracking: Float,
    /** Whether this row is for something a machine produced. */
    val mono: Boolean,
) {
    companion object {
        private fun of(size: Float, weight: Int, em: Float, mono: Boolean = false) =
            GlanceType(size, weight, em * size, mono)

        /** 17 / 600, −0.016em. A headline, a widget's primary count label. */
        val headline = of(17f, 600, -0.016f)

        /** 15 / 600, −0.014em. A card header, a column row's title. */
        val cardHeader = of(15f, 600, -0.014f)

        /** 13 / 600, −0.010em. Workspace names in rows. */
        val rowName = of(13f, 600, -0.010f)

        /**
         * 12.5 / 400, no tracking. Terminal output in the app. Mono, because it
         * is the most literal case of "it came off a machine" in the product.
         *
         * Not used by `ui/TerminalCanvas.kt`, and that is not an oversight: the
         * terminal's own size is a user setting in `data/Settings.kt`, which is
         * a promise this app made before the spec existed and is a stronger
         * claim than a default. This row is the default a surface uses when it
         * has no such setting to consult.
         */
        val terminal = of(12.5f, 400, 0f, mono = true)

        /**
         * 11 / 400, no tracking. Secondary lines, notes, and all mono figures.
         *
         * **The absolute floor**, and §01's contrast rule leans on it: "nothing
         * below L 0.7 on the card and nothing below 11px. The brief's test is
         * one second at arm's length in sunlight; treat both numbers as hard."
         */
        val secondary = of(11f, 400, 0f)

        /** The same size and weight, in mono — timestamps, counts, diff stats. */
        val monoFigures = of(11f, 400, 0f, mono = true)

        /**
         * 38–46 / 600, −0.035em, tabular. The single count on a widget.
         *
         * A RANGE in the spec rather than one figure, because the number it
         * draws is one or two digits depending on the fleet and the tile it sits
         * in is the same size either way. The caller picks inside the range;
         * anything outside it is clamped, so a family that wants a bigger number
         * gets the biggest one this system has rather than one nobody specified.
         */
        fun count(size: Float = 44f) = of(min(46f, max(38f, size)), 600, -0.035f)
    }
}

/**
 * How old a snapshot is, in the two words §02 allows.
 *
 * **No clock, anywhere.** §02: "iOS prints the time 20pt above every one of
 * these surfaces. Where a clock would go, print the age of the snapshot."
 *
 * **And never a running one.** "Relative and coarse: 2m ago, 52m ago, 3h ago.
 * Never a running clock on an idle agent — precision nobody needs implies
 * precision we do not have."
 *
 * Deliberately NOT a replacement for [Terminal.rowStatus]'s duration, which
 * ticks once a second and is right to: that is a wait you are actively in — an
 * agent blocked for twenty minutes is a different situation from one blocked for
 * ten seconds — and `Terminal.hasClock` is already the gate that says which
 * terminals earn it.
 */
object GlanceAge {
    /** "2m", "52m", "3h" — the bare figure, for a slot that already separates it. */
    fun brief(millis: Long): String {
        val seconds = max(0L, millis) / 1000
        if (seconds < 60) return "now"
        if (seconds < 3600) return "${seconds / 60}m"
        if (seconds < 86_400) return "${seconds / 3600}h"
        return "${seconds / 86_400}d"
    }

    /**
     * "2m ago", "52m ago", "3h ago" — the sentence form, where the surface has
     * the room. "now" keeps no "ago", which would be a contradiction.
     */
    fun stated(millis: Long): String {
        val brief = brief(millis)
        return if (brief == "now") "just now" else "$brief ago"
    }

    /**
     * Under two minutes, which is §08's definition of a fresh snapshot.
     *
     * Deliberately NOT [GlanceMark.STALE_AFTER_MS], which is an hour and answers
     * a different question — whether a claim about the present may still be
     * asserted at all. Two minutes is when a surface starts saying how old it
     * is; an hour is when it stops vouching.
     */
    const val FRESH_MS = 120L * 1000L
}
