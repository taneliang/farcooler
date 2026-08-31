package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The glance vocabulary, pinned.
 *
 * ## Where the expected colours come from
 *
 * **Not from this file's own arithmetic.** A test that re-implemented
 * [Oklch.toArgb]'s matrices to check [Oklch.toArgb] would pass with both copies
 * transposed the same way, which is precisely the failure it exists to catch.
 *
 * Every hex below was read out of Chrome's own CSS Color 4 implementation, by
 * setting a canvas `fillStyle` to the spec's literal `oklch()` string and
 * reading the rasterized pixel back:
 *
 * ```
 * x.fillStyle = "oklch(0.78 0.16 70)"; x.fillRect(0,0,1,1);
 * x.getImageData(0,0,1,1).data  // → 247,162,36
 * ```
 *
 * So the ground truth is a second, independently written implementation of the
 * same standard, fed the same strings the design document prints. Where a value
 * here disagrees with Chrome by more than one unit in a channel, this port is
 * wrong.
 *
 * The tolerance is [TOLERANCE] and it is one, for rounding at the last step
 * only: Chrome rounds the transfer function's output to a byte and so does
 * [Oklch.toArgb], but they may sit either side of a half.
 */
class GlanceTest {

    /** One unit in a channel, for the rounding at the last step. See the class doc. */
    private val TOLERANCE = 1

    private fun assertRgb(expected: Triple<Int, Int, Int>, actual: Int, what: String) {
        val r = (actual shr 16) and 0xFF
        val g = (actual shr 8) and 0xFF
        val b = actual and 0xFF
        val (er, eg, eb) = expected
        assertTrue(
            "$what: expected rgb($er, $eg, $eb) from Chrome, got rgb($r, $g, $b)",
            Math.abs(r - er) <= TOLERANCE &&
                Math.abs(g - eg) <= TOLERANCE &&
                Math.abs(b - eb) <= TOLERANCE,
        )
    }

    // MARK: - §01, against Chrome

    @Test
    fun `the twelve colours match Chrome's own oklch conversion`() {
        assertRgb(Triple(247, 162, 36), GlancePalette.amber.dark.toArgb(), "amber dark")
        assertRgb(Triple(185, 117, 21), GlancePalette.amber.light.toArgb(), "amber light")
        assertRgb(Triple(144, 183, 207), GlancePalette.review.dark.toArgb(), "review dark")
        assertRgb(Triple(63, 105, 129), GlancePalette.review.light.toArgb(), "review light")
        assertRgb(Triple(214, 215, 217), GlancePalette.code.dark.toArgb(), "code dark")
        assertRgb(Triple(40, 41, 42), GlancePalette.code.light.toArgb(), "code light")
        assertRgb(Triple(133, 134, 135), GlancePalette.chat.dark.toArgb(), "chat dark")
        assertRgb(Triple(98, 99, 100), GlancePalette.chat.light.toArgb(), "chat light")
        assertRgb(Triple(241, 242, 243), GlancePalette.commit.darkArgb, "commit")
        assertRgb(Triple(82, 83, 84), GlancePalette.axis.darkArgb, "axis")
        assertRgb(Triple(76, 77, 78), GlancePalette.empty.darkArgb, "empty")
        assertRgb(Triple(244, 245, 246), GlancePalette.text1.darkArgb, "text 1")
        assertRgb(Triple(169, 171, 173), GlancePalette.text2.darkArgb, "text 2")
        // The three surfaces carry alpha, which a canvas pixel cannot report
        // without compositing, so the ground truth above was read from the
        // opaque form of each. Alpha is asserted separately below.
        assertRgb(Triple(25, 27, 28), GlancePalette.card.dark.copy(alpha = 1.0).toArgb(), "card")
        assertRgb(
            Triple(18, 20, 22), GlancePalette.widget.dark.copy(alpha = 1.0).toArgb(), "widget")
        assertRgb(Triple(5, 6, 6), GlancePalette.island.dark.toArgb(), "island")
    }

    @Test
    fun `alpha survives the conversion`() {
        assertEquals(230, (GlancePalette.card.darkArgb ushr 24) and 0xFF) // 0.9
        assertEquals(242, (GlancePalette.widget.darkArgb ushr 24) and 0xFF) // 0.95
        assertEquals(255, (GlancePalette.island.darkArgb ushr 24) and 0xFF)
    }

    /**
     * §01: "Not a filter flip." Light mode is a different palette, so the four
     * values that have one must actually differ — a port that let the
     * two-argument constructor decay into the one-argument one would still
     * compile, still draw, and quietly lose half the spec.
     */
    @Test
    fun `light mode is a second palette and not the dark one dimmed`() {
        assertNotEquals(GlancePalette.amber.darkArgb, GlancePalette.amber.argb(dark = false))
        assertNotEquals(GlancePalette.review.darkArgb, GlancePalette.review.argb(dark = false))
        // The trace tones INVERT: code is the bright half in dark mode and the
        // dark half in light mode, and chat does the opposite. A port that
        // copied the dark column into both would pass an equality test and fail
        // this one.
        assertTrue(
            "code must be brighter than chat in dark mode",
            luminance(GlancePalette.code.darkArgb) > luminance(GlancePalette.chat.darkArgb),
        )
        assertTrue(
            "code must be darker than chat in light mode",
            luminance(GlancePalette.code.argb(dark = false)) <
                luminance(GlancePalette.chat.argb(dark = false)),
        )
    }

    /**
     * The neutrals are neutral. Nine of the twelve values sit at chroma 0.002 to
     * 0.004, which is a grey; a transposed matrix in [Oklch.toArgb] shows up
     * here as a channel drifting away from its neighbours long before it is
     * visible on a screen.
     */
    @Test
    fun `the neutrals stay neutral`() {
        for ((name, ink) in listOf(
            "commit" to GlancePalette.commit,
            "axis" to GlancePalette.axis,
            "empty" to GlancePalette.empty,
            "text1" to GlancePalette.text1,
            "text2" to GlancePalette.text2,
        )) {
            val argb = ink.darkArgb
            val r = (argb shr 16) and 0xFF
            val g = (argb shr 8) and 0xFF
            val b = argb and 0xFF
            assertTrue("$name should be a near-grey, got $r/$g/$b", maxOf(r, g, b) - minOf(r, g, b) <= 6)
        }
    }

    /**
     * Amber is the warm one and review is the cool one, in the only channel
     * ordering that says so. A sign error on the `b` component of the Oklab
     * polar conversion swaps exactly this and nothing else.
     */
    @Test
    fun `amber is warm and review is cool`() {
        val amber = GlancePalette.amber.darkArgb
        assertTrue(
            "amber must run red > green > blue",
            ((amber shr 16) and 0xFF) > ((amber shr 8) and 0xFF) &&
                ((amber shr 8) and 0xFF) > (amber and 0xFF),
        )
        val review = GlancePalette.review.darkArgb
        assertTrue(
            "review must run blue > green > red",
            (review and 0xFF) > ((review shr 8) and 0xFF) &&
                ((review shr 8) and 0xFF) > ((review shr 16) and 0xFF),
        )
    }

    private fun luminance(argb: Int): Int =
        ((argb shr 16) and 0xFF) + ((argb shr 8) and 0xFF) + (argb and 0xFF)

    // MARK: - §03, the mark

    /**
     * The stroke table, read straight out of §03's three ladders. Every one of
     * these twelve numbers is a literal in the spec; none is derived, and the
     * whole vocabulary is half-point differences between them.
     */
    @Test
    fun `the stroke ladder is the spec's`() {
        val needsYou = GlanceMark.Attention.NEEDS_YOU
        val toReview = GlanceMark.Attention.TO_REVIEW
        val quiet = GlanceMark.Attention.QUIET

        assertEquals(2f, GlanceMarkSize.RIBBON.stroke(needsYou))
        assertEquals(2.5f, GlanceMarkSize.ROW.stroke(needsYou))
        assertEquals(2.5f, GlanceMarkSize.HEADER.stroke(needsYou))
        assertEquals(3.5f, GlanceMarkSize.LONE.stroke(needsYou))

        assertEquals(2f, GlanceMarkSize.RIBBON.stroke(toReview))
        assertEquals(2f, GlanceMarkSize.ROW.stroke(toReview))
        assertEquals(2f, GlanceMarkSize.HEADER.stroke(toReview))
        assertEquals(3f, GlanceMarkSize.LONE.stroke(toReview))

        for (size in GlanceMarkSize.entries) {
            assertEquals("hairline is 1 at every size", 1f, size.stroke(quiet))
        }
    }

    /**
     * §03: "there is no room for four distinguishable strokes, so an 8pt ribbon
     * separates wants you from quiet and leaves amber-versus-review to hue."
     */
    @Test
    fun `the two attention tiers collapse to one weight at 8dp and part above it`() {
        assertEquals(
            GlanceMarkSize.RIBBON.stroke(GlanceMark.Attention.NEEDS_YOU),
            GlanceMarkSize.RIBBON.stroke(GlanceMark.Attention.TO_REVIEW),
        )
        assertNotEquals(
            GlanceMarkSize.ROW.stroke(GlanceMark.Attention.NEEDS_YOU),
            GlanceMarkSize.ROW.stroke(GlanceMark.Attention.TO_REVIEW),
        )
    }

    /** §03: "3 at 8pt, 4 at 11pt, 5 at 15pt. There is no 10pt core." */
    @Test
    fun `the core table has a hole in it at 10dp`() {
        assertEquals(3f, GlanceMarkSize.RIBBON.core)
        assertNull("a row shows the ring alone", GlanceMarkSize.ROW.core)
        assertEquals(4f, GlanceMarkSize.HEADER.core)
        assertEquals(5f, GlanceMarkSize.LONE.core)
    }

    /**
     * A core is never wider than the hole its ring leaves. §03's own reason for
     * the missing 10dp core — "a 3.5pt disc inside a 5pt hole … reads as a
     * smudge" — is the general form of this, and it is worth asserting for every
     * size rather than trusting four numbers to keep agreeing.
     */
    @Test
    fun `every core fits inside its ring`() {
        for (size in GlanceMarkSize.entries) {
            val core = size.core ?: continue
            for (attention in GlanceMark.Attention.entries) {
                val hole = size.diameter - 2 * size.stroke(attention)
                assertTrue(
                    "$size $attention: core $core does not fit in a $hole hole",
                    core < hole,
                )
            }
        }
    }

    /**
     * The mark's OUTER edge lands on the diameter §03 names, for every size and
     * every tier.
     *
     * This is the `strokeBorder`-versus-`stroke` mistake, asserted. A stroke
     * straddles its path, so a ring centred on `diameter / 2` sticks out by half
     * its own width — and by a DIFFERENT amount per tier, since the tiers are
     * what the stroke ladder varies. A column of row marks would stop lining up
     * the moment one agent got blocked, which is a defect nobody would think to
     * look for and which this arithmetic makes impossible.
     */
    @Test
    fun `the ring is drawn inside the diameter, not straddling it`() {
        for (size in GlanceMarkSize.entries) {
            for (attention in GlanceMark.Attention.entries) {
                val outer = size.ringRadius(attention) + size.stroke(attention) / 2
                assertEquals(
                    "$size $attention: outer edge must be at ${size.diameter / 2}",
                    size.diameter / 2,
                    outer,
                    1e-5f,
                )
            }
        }
    }

    /**
     * One dash RULE rather than one dash array, and the reason is the heavy end:
     * a fixed pattern that reads correctly at a 1dp hairline turns a 3.5dp ring
     * into a ring of bars wider than they are long — a sunburst, which is to say
     * a spinner, which is to say the drawing for "unreachable" reading as
     * "working". So the segment must stay longer than it is wide at every stroke
     * in the ladder.
     */
    @Test
    fun `the dash segment stays longer than the stroke is wide at every size`() {
        for (size in GlanceMarkSize.entries) {
            for (attention in GlanceMark.Attention.entries) {
                val stroke = size.stroke(attention)
                val (on, off) = GlanceMark.dash(stroke).let { it[0] to it[1] }
                assertTrue("$size $attention: $on must exceed $stroke", on > stroke)
                assertEquals("the gap matches the stroke", stroke, off, 1e-5f)
            }
        }
    }

    /**
     * A dash that did not scale with the stroke is the failure above, written
     * the other way round: two different rings would wear the same pattern, and
     * the heavier one would be the one that broke.
     */
    @Test
    fun `the dash scales with the stroke rather than being a constant`() {
        assertNotEquals(
            GlanceMark.dash(1f).toList(),
            GlanceMark.dash(3.5f).toList(),
        )
    }

    // MARK: - What a mark is made of

    private fun terminal(activity: String, since: Double? = null, failed: Boolean? = null) =
        Terminal(id = "t", activity = activity, activitySince = since, turnFailed = failed)

    @Test
    fun `blocked is the heavy ring and no core`() {
        val mark = GlanceMark.of(terminal("blocked"), now = 0L)!!
        assertEquals(GlanceMark.Attention.NEEDS_YOU, mark.attention)
        assertEquals(GlanceMark.Core.AT_A_PROMPT, mark.core)
        assertEquals(GlanceMark.Link.LIVE, mark.link)
    }

    @Test
    fun `working is a hairline with a filled core`() {
        val mark = GlanceMark.of(terminal("working"), now = 0L)!!
        assertEquals(GlanceMark.Attention.QUIET, mark.attention)
        assertEquals(GlanceMark.Core.PRODUCING, mark.core)
    }

    /**
     * **A finished turn has no mark, and that is a refusal rather than a gap.**
     *
     * An earlier version of this file mapped `done` onto the amber ring, on the
     * grounds that `AgentActivity.wantsAttention` is blocked OR done — this
     * app's single definition of "interrupt someone" — and that the row prints
     * the word beside the mark anyway. Both halves are true and the conclusion
     * was still wrong: the second half holds on a fleet row and fails on a tab
     * chip, which has no state text at all, and folding a failed turn into amber
     * put back the exact defect red was introduced to fix.
     *
     * The Mac reached the opposite conclusion first, in `Status.glanceMark`, and
     * three platforms giving three answers to one question is what this port
     * exists to end. So: no mark, and [agentOutcome] answers instead.
     */
    @Test
    fun `a finished turn has no mark and an outcome instead`() {
        assertNull(GlanceMark.of(terminal("done"), now = 0L))
        assertEquals(AgentOutcome.DONE, agentOutcome(terminal("done")))
    }

    /**
     * Red exists precisely because a turn that died and a turn that worked were
     * the same dot until it did. The daemon sends both as `done` and says which
     * in `turnFailed`, so this is the one distinction a `when` on the activity
     * alone cannot make — and the one a chip's hue carries by itself.
     */
    @Test
    fun `a turn that died is a different outcome from one that worked`() {
        assertEquals(AgentOutcome.FAILED, agentOutcome(terminal("done", failed = true)))
        assertNotEquals(
            agentOutcome(terminal("done")),
            agentOutcome(terminal("done", failed = true)),
        )
        assertNull(GlanceMark.of(terminal("done", failed = true), now = 0L))
    }

    /**
     * **Exactly one of the two answers, for every agent pane.** [GlanceMark.of]
     * and [agentOutcome] are two `when`s in one file today and will be read by
     * three surfaces; nothing but this test stops them drifting into a state
     * that draws two dots or none.
     *
     * `turnFailed` is only meaningful on `done` — `Terminal.turnDidFail` gates
     * on it — so the loop covers the flag against every activity precisely to
     * catch a future edit that starts honouring it somewhere else.
     */
    @Test
    fun `every agent pane gets exactly one of a mark and an outcome`() {
        for (activity in listOf("idle", "working", "blocked", "done", "unknown", "wat")) {
            for (failed in listOf(false, true)) {
                val t = terminal(activity, failed = failed)
                val mark = GlanceMark.of(t, now = 0L)
                val outcome = agentOutcome(t)
                assertTrue(
                    "$activity failed=$failed: mark=$mark outcome=$outcome",
                    (mark == null) != (outcome == null),
                )
            }
        }
    }

    /** A pane with no agent in it says nothing at all, in either channel. */
    @Test
    fun `a pane that is not an agent has neither`() {
        val shell = Terminal(id = "t")
        assertNull(agentOutcome(shell))
        // The mark still answers for a plain shell — it is the quiet hairline,
        // which is true of it — but no surface draws one: every call site guards
        // on `agent.isAgent` first, the way the fleet row does.
        assertEquals(GlanceMark.Attention.QUIET, GlanceMark.of(shell, now = 0L)?.attention)
    }

    /** §08: "Blocked and to-review hold at any age; working and idle go dashed." */
    @Test
    fun `staleness dashes a quiet ring and never an attention one`() {
        val now = 1_800_000_000_000L
        val longAgo = (now - 3 * GlanceMark.STALE_AFTER_MS).toDouble()
        assertEquals(
            GlanceMark.Link.BROKEN,
            GlanceMark.of(terminal("working", since = longAgo), now)!!.link,
        )
        assertEquals(
            GlanceMark.Link.LIVE,
            GlanceMark.of(terminal("blocked", since = longAgo), now)!!.link,
        )
    }

    /**
     * Null means "not told", which is a different thing from "told a long time
     * ago". An older daemon sends no timestamp for anything, and reading that as
     * silence would draw a whole healthy fleet as unreachable.
     */
    @Test
    fun `a terminal with no timestamp is not stale`() {
        assertEquals(
            GlanceMark.Link.LIVE,
            GlanceMark.of(terminal("working", since = null), now = Long.MAX_VALUE / 2)!!.link,
        )
    }

    @Test
    fun `only an unread diff earns the review tier`() {
        val row = { changed: Boolean, insertions: Int ->
            InboxRow(workspaceId = "w", changedSinceReviewed = changed, insertions = insertions)
        }
        assertEquals(
            GlanceMark.Attention.TO_REVIEW,
            GlanceMark.ofDiff(row(true, 12)).attention,
        )
        // Read already: there is a branch here, but nothing new.
        assertEquals(GlanceMark.Attention.QUIET, GlanceMark.ofDiff(row(false, 12)).attention)
        // Changed, but empty: nothing to look at.
        assertEquals(GlanceMark.Attention.QUIET, GlanceMark.ofDiff(row(true, 0)).attention)
        assertEquals(GlanceMark.Attention.QUIET, GlanceMark.ofDiff(null).attention)
    }

    /**
     * An agent tab can never be cyan. `InboxRow` is a WORKSPACE's counts, not an
     * agent's state, and inventing a per-agent version of it would sort a diff
     * by how blocked some agent in the same worktree happens to be.
     */
    @Test
    fun `no terminal can produce the review tier`() {
        for (activity in listOf("none", "idle", "working", "blocked", "done", "unknown", "wat")) {
            assertNotEquals(
                "$activity must not be able to draw the review ring",
                GlanceMark.Attention.TO_REVIEW,
                GlanceMark.of(terminal(activity), now = 0L)?.attention,
            )
        }
    }

    // MARK: - Words

    /**
     * The whole mark is a difference in line width, fill and dash, which is to
     * say the whole mark is invisible to TalkBack unless it is also said out
     * loud. Twelve states, twelve distinguishable phrases — except the four a
     * broken ring collapses, which is correct: an unreachable agent's core is
     * not a claim anyone should be reading aloud.
     */
    @Test
    fun `every drawn state says something and no two live ones say the same thing`() {
        val live = GlanceMark.Attention.entries.flatMap { attention ->
            GlanceMark.Core.entries.map { GlanceMark(attention, it) }
        }
        assertEquals(6, live.map { it.phrase }.toSet().size)
        assertEquals("Needs you, at a prompt", GlanceMark(GlanceMark.Attention.NEEDS_YOU, GlanceMark.Core.AT_A_PROMPT).phrase)
        assertEquals("Nothing wanted, producing", GlanceMark(GlanceMark.Attention.QUIET, GlanceMark.Core.PRODUCING).phrase)
        assertEquals(
            "To review, unreachable",
            GlanceMark(GlanceMark.Attention.TO_REVIEW, GlanceMark.Core.PRODUCING, GlanceMark.Link.BROKEN).phrase,
        )
    }

    /**
     * A surface declining to state the core must not have that read out as "at a
     * prompt" — which is a claim, and the one `core == null` exists to withhold.
     */
    @Test
    fun `a withheld core says nothing rather than saying at a prompt`() {
        val quiet = GlanceMark(GlanceMark.Attention.QUIET, GlanceMark.Core.AT_A_PROMPT)
        assertEquals("Nothing wanted", quiet.withoutCore.phrase)
        assertNotEquals(quiet.phrase, quiet.withoutCore.phrase)
    }

    // MARK: - §02

    @Test
    fun `tracking is the spec's em figure resolved at each style's own size`() {
        assertEquals(17f * -0.016f, GlanceType.headline.tracking, 1e-6f)
        assertEquals(13f * -0.010f, GlanceType.rowName.tracking, 1e-6f)
        assertEquals(0f, GlanceType.secondary.tracking, 1e-6f)
    }

    @Test
    fun `nothing in the ramp goes below the 11 floor`() {
        for (style in listOf(
            GlanceType.headline,
            GlanceType.cardHeader,
            GlanceType.rowName,
            GlanceType.terminal,
            GlanceType.secondary,
            GlanceType.monoFigures,
            GlanceType.count(),
        )) {
            assertTrue("${style.size} is below the floor", style.size >= 11f)
        }
    }

    @Test
    fun `the widget count clamps into its range rather than taking what it is given`() {
        assertEquals(38f, GlanceType.count(12f).size)
        assertEquals(46f, GlanceType.count(200f).size)
        assertEquals(41f, GlanceType.count(41f).size)
    }

    @Test
    fun `machine figures are mono and words are not`() {
        assertTrue(GlanceType.monoFigures.mono)
        assertTrue(GlanceType.terminal.mono)
        assertTrue(!GlanceType.rowName.mono)
        assertTrue(!GlanceType.headline.mono)
    }

    // MARK: - Age

    @Test
    fun `age is relative and coarse`() {
        assertEquals("now", GlanceAge.brief(1_000))
        assertEquals("2m", GlanceAge.brief(120_000))
        assertEquals("52m", GlanceAge.brief(52 * 60_000L))
        assertEquals("3h", GlanceAge.brief(3 * 3_600_000L))
        assertEquals("2d", GlanceAge.brief(2 * 86_400_000L))
        assertEquals("just now", GlanceAge.stated(1_000))
        assertEquals("52m ago", GlanceAge.stated(52 * 60_000L))
    }

    /**
     * Two minutes and an hour answer different questions, and a port that reused
     * one constant for both would make a surface stop vouching for a working
     * agent after two minutes.
     */
    @Test
    fun `fresh and stale are different thresholds`() {
        assertNotEquals(GlanceAge.FRESH_MS, GlanceMark.STALE_AFTER_MS)
        assertTrue(GlanceAge.FRESH_MS < GlanceMark.STALE_AFTER_MS)
    }
}
