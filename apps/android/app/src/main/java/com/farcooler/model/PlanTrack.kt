package com.farcooler.model

/**
 * The track line (ov-331): one plain-words line per theme that says whether the work is moving.
 * A port of `PlanThemeTrack.swift`, which holds the design (`.claude/agent/reports/theme-detail/design.md`, 2.2);
 * the words and rules are held to the same cases by `PlanTrackTest`.
 *
 * Android has no card board to read, so a theme's last move is the runner's own word (`last_moved_at`, which
 * counts its story, lanes, rulings and cards) when it sent one, and what the plan alone shows otherwise.
 *
 * Amber is for the owner's to act on: an ask, and a budget gone over. A track line is amber only for the latter.
 */

/** How long a theme sits with no lane and nothing moving before it reads as quiet: a day (the owner's ruling D3). */
const val PLAN_QUIET_AFTER_MS = 24L * 3_600_000

/** The working lane that has sat longest in one state, named beside lanes that are moving. */
data class PlanTrackStalled(val lane: String, val since: Long)

/** A lane moving a theme, as the track line names it. */
data class PlanTrackLane(val name: String, val state: LaneState, val fixRounds: Int)

/** Where a theme stands in motion: the first rule that matches. */
sealed interface PlanTrack {
    /** Past its token budget. */
    data class OverBudget(val budget: PlanBudget) : PlanTrack
    /** Every lane working it has sat in one state for over an hour: the one that has sat longest is named. */
    data class Stuck(val lane: String, val since: Long) : PlanTrack
    /**
     * At least one lane is building, in review, fixing or landing it, and is still moving. A lane that has sat over an
     * hour beside them is named after ([stalled]): the others are moving, and the line says so first.
     */
    data class Moving(val lanes: List<PlanTrackLane>, val stalled: PlanTrackStalled? = null) : PlanTrack
    /** No lane is working it and one is queued for it. */
    data class Queued(val rank: Int?) : PlanTrack
    /** Open cards, no lane, and nothing moved for a day or more. */
    data class Quiet(val since: Long) : PlanTrack
    /** Open cards, no lane, and something moved within the day. */
    data object Idle : PlanTrack
    /** Every card is done. */
    data object AllDone : PlanTrack
    data object Paused : PlanTrack
    data object Done : PlanTrack

    /** Amber, and only this: a budget gone over. */
    val needsAttention: Boolean get() = this is OverBudget
    val isMoving: Boolean get() = this is Moving || this is Stuck
    val isQuiet: Boolean get() = this is Quiet
}

/**
 * When [theme] last moved: the newest of its story, the runner's word, its lanes (a dropped lane is gone) and its
 * rulings, made or settled.
 */
fun Plan.lastMoved(theme: PlanTheme): Long {
    val tasks = theme.cards.map { it.task }.toSet()
    var at = maxOf(theme.storyAt, theme.lastMovedAt ?: 0L)
    for (lane in lanes) {
        if (lane.state != LaneState.DROPPED && lane.cards.any { it.task in tasks }) at = maxOf(at, lane.stateSince)
    }
    for (ruling in rulings) {
        if (ruling.themeId == theme.id) at = maxOf(at, ruling.createdAt, ruling.settledAt ?: 0L)
    }
    return at
}

/** Where [theme] stands in motion, as of the runner's clock. */
fun Plan.track(theme: PlanTheme): PlanTrack {
    if (theme.state == "paused") return PlanTrack.Paused
    if (theme.state == "done") return PlanTrack.Done
    PlanCostWords.overBudget(theme)?.let { return PlanTrack.OverBudget(it) }
    val mine = lanesIn(theme)
    val working = mine.filter { it.state.isLive && it.state != LaneState.QUEUED }
    // The lane stalled longest, not the first in plan order; it stands alone only when nothing else is moving (an hour
    // is routine for a build, so it never hides the lanes that are).
    val stalled = working.filter { it.stale }.minByOrNull { it.stateSince }
    if (stalled != null && working.all { it.stale }) return PlanTrack.Stuck(stalled.heading, stalled.stateSince)
    if (working.isNotEmpty()) {
        return PlanTrack.Moving(
            working.map { PlanTrackLane(it.heading, it.state, it.fixRounds) },
            stalled?.let { PlanTrackStalled(it.heading, it.stateSince) },
        )
    }
    val queued = mine.filter { it.state == LaneState.QUEUED }
    if (queued.isNotEmpty()) return PlanTrack.Queued(queued.mapNotNull { it.planRank }.minOrNull())
    val total = PlanWords.total(theme.counts)
    if (total > 0 && theme.counts.done == total) return PlanTrack.AllDone
    val moved = lastMoved(theme)
    if (total > theme.counts.done && moved > 0 && nowMs - moved >= PLAN_QUIET_AFTER_MS) return PlanTrack.Quiet(moved)
    return PlanTrack.Idle
}

/** The themes as a list draws them: active ones in the board's order, then paused and done ones folded into one row. */
data class PlanThemeGroups(val active: List<PlanTheme>, val closed: List<PlanTheme>)

/** The Mac's Themes, the iPhone's and Android's all split on this one rule. */
val Plan.themeGroups: PlanThemeGroups
    get() = shownThemes.let { shown -> PlanThemeGroups(shown.filter { it.state == "active" }, shown.filter { it.state != "active" }) }

/** "3 waiting on you · 4 moving · 1 quiet": the Themes section's one line, or null when none of it applies. */
fun Plan.trackSummary(): String? {
    val shown = shownThemes
    // Only active themes: a paused theme's ask is inside the closed fold, and Needs You lists it.
    val active = shown.filter { it.state == "active" }
    val asking = active.count { it.ownerAsk.isNotEmpty() }
    val tracks = active.map { track(it) }
    val parts = listOf(asking to "waiting on you", tracks.count { it.isMoving } to "moving", tracks.count { it.isQuiet } to "quiet")
        .filter { it.first > 0 }.map { "${it.first} ${it.second}" }
    return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
}

/** "2 lanes moving", or "fix-gestures is fixing, round 1": a lane named when one is the whole story. */
fun PlanWords.track(track: PlanTrack, now: Long): String = when (track) {
    is PlanTrack.OverBudget -> PlanCostWords.budgetLine(track.budget)
    is PlanTrack.Stuck -> "${track.lane}: ${noMove(track.since, now)}"
    is PlanTrack.Moving -> {
        val tail = track.stalled?.let { " · ${it.lane}: ${noMove(it.since, now)}" } ?: ""
        val lane = track.lanes.singleOrNull()
        if (lane == null) {
            "${track.lanes.size} lanes moving$tail"
        } else {
            val verb = when (lane.state) {
                LaneState.BUILDING -> "building"
                LaneState.REVIEW -> "in review"
                LaneState.FIXING -> if (lane.fixRounds > 0) "fixing, round ${lane.fixRounds}" else "fixing"
                LaneState.LANDING -> "landing"
                else -> "moving"
            }
            "${lane.name} is $verb$tail"
        }
    }
    is PlanTrack.Queued -> track.rank?.let { "Queued, ${ordinal(it)} up" } ?: "Queued"
    is PlanTrack.Quiet -> {
        val days = maxOf(1L, maxOf(0L, now - track.since) / 86_400_000)
        if (days == 1L) "No lane · quiet for 1 day" else "No lane · quiet for $days days"
    }
    PlanTrack.Idle -> "No lane yet"
    PlanTrack.AllDone -> "Every card is done"
    PlanTrack.Paused -> "Paused"
    PlanTrack.Done -> "Done"
}

/** "no move in an hour", "no move in 3 h". */
private fun noMove(since: Long, now: Long): String {
    val minutes = maxOf(0L, now - since) / 60_000
    return if (minutes < 120) "no move in an hour" else "no move in ${minutes / 60} h"
}

/** The same for TalkBack: a middle dot isn't read well. */
fun PlanWords.trackSpoken(track: PlanTrack, now: Long): String = when (track) {
    is PlanTrack.OverBudget -> PlanCostWords.budgetSpoken(track.budget)
    else -> track(track, now).replace(" · ", ", ")
}

/** "Updated 1 h ago": the story's age, printed and never judged. */
fun PlanWords.storyAge(theme: PlanTheme, now: Long): String? =
    if (theme.storyAt > 0 && theme.story.isNotEmpty()) "Updated ${ago(theme.storyAt, now)}" else null

/** How many lines a theme's story gets on a phone's row (ov-331). */
const val PLAN_STORY_LINES = 3
