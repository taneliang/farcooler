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

/** A lane moving a theme, as the track line names it. */
data class PlanTrackLane(val name: String, val state: LaneState, val fixRounds: Int)

/** Where a theme stands in motion: the first rule that matches. */
sealed interface PlanTrack {
    /** Past its token budget. */
    data class OverBudget(val budget: PlanBudget) : PlanTrack
    /** A live lane has sat in one state for over an hour. */
    data class Stuck(val lane: String, val since: Long) : PlanTrack
    /** At least one lane is building, in review, fixing or landing it. */
    data class Moving(val lanes: List<PlanTrackLane>) : PlanTrack
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

/** When [theme] last moved: the newest of its story, the runner's word and its lanes (a dropped lane is gone). */
fun Plan.lastMoved(theme: PlanTheme): Long {
    val tasks = theme.cards.map { it.task }.toSet()
    var at = maxOf(theme.storyAt, theme.lastMovedAt ?: 0L)
    for (lane in lanes) {
        if (lane.state != LaneState.DROPPED && lane.cards.any { it.task in tasks }) at = maxOf(at, lane.stateSince)
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
    working.firstOrNull { it.stale }?.let { return PlanTrack.Stuck(it.name, it.stateSince) }
    if (working.isNotEmpty()) return PlanTrack.Moving(working.map { PlanTrackLane(it.name, it.state, it.fixRounds) })
    val queued = mine.filter { it.state == LaneState.QUEUED }
    if (queued.isNotEmpty()) return PlanTrack.Queued(queued.mapNotNull { it.planRank }.minOrNull())
    val total = PlanWords.total(theme.counts)
    if (total > 0 && theme.counts.done == total) return PlanTrack.AllDone
    val moved = lastMoved(theme)
    if (total > theme.counts.done && moved > 0 && nowMs - moved >= PLAN_QUIET_AFTER_MS) return PlanTrack.Quiet(moved)
    return PlanTrack.Idle
}

/** "3 waiting on you · 4 moving · 1 quiet": the Themes section's one line, or null when none of it applies. */
fun Plan.trackSummary(): String? {
    val shown = shownThemes
    val asking = shown.count { it.ownerAsk.isNotEmpty() && it.state != "done" }
    val tracks = shown.filter { it.state == "active" }.map { track(it) }
    val parts = listOf(asking to "waiting on you", tracks.count { it.isMoving } to "moving", tracks.count { it.isQuiet } to "quiet")
        .filter { it.first > 0 }.map { "${it.first} ${it.second}" }
    return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
}

/** "2 lanes moving", or "fix-gestures is fixing, round 1": a lane named when one is the whole story. */
fun PlanWords.track(track: PlanTrack, now: Long): String = when (track) {
    is PlanTrack.OverBudget -> PlanCostWords.budgetLine(track.budget)
    is PlanTrack.Stuck -> {
        val minutes = maxOf(0L, now - track.since) / 60_000
        if (minutes < 120) "${track.lane}: no move in an hour" else "${track.lane}: no move in ${minutes / 60} h"
    }
    is PlanTrack.Moving -> {
        val lane = track.lanes.singleOrNull()
        if (lane == null) {
            "${track.lanes.size} lanes moving"
        } else {
            val verb = when (lane.state) {
                LaneState.BUILDING -> "building"
                LaneState.REVIEW -> "in review"
                LaneState.FIXING -> if (lane.fixRounds > 0) "fixing, round ${lane.fixRounds}" else "fixing"
                LaneState.LANDING -> "landing"
                else -> "moving"
            }
            "${lane.name} is $verb"
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
