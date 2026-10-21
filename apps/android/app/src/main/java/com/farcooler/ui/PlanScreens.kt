package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Bedtime
import androidx.compose.material.icons.outlined.Build
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.Circle
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.PauseCircleOutline
import androidx.compose.material.icons.outlined.Sync
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.QuestionMark
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VerticalAlignBottom
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.material3.LocalContentColor
import com.farcooler.model.shownStage
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.GlancePalette
import com.farcooler.model.LaneState
import com.farcooler.model.Plan
import com.farcooler.model.PlanCostWords
import com.farcooler.model.PlanCounts
import com.farcooler.model.PlanFleetAgent
import com.farcooler.model.PlanGlanceSentence
import com.farcooler.model.PlanSections
import com.farcooler.model.PlanLane
import com.farcooler.model.ciOf
import com.farcooler.model.nowGroups
import com.farcooler.model.PlanPage
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanSegment
import com.farcooler.model.PLAN_STORY_LINES
import com.farcooler.model.PlanTheme
import com.farcooler.model.PlanTrack
import com.farcooler.model.storyAge
import com.farcooler.model.themeGroups
import com.farcooler.model.track
import com.farcooler.model.trackSpoken
import com.farcooler.model.trackSummary
import com.farcooler.model.PlanWords
import com.farcooler.model.TaskStatus

// The Plan view on Android (ov-268 design 6.5): what the board's list shows
// when Plan is chosen. Next up first, because it's what the owner can't see
// anywhere else; then the lanes working now, the themes, and what landed
// today. A lane or a theme opens its page, pushed.
//
// EXPERIMENTAL and opt-in, as the layer is: Tasks is the default, the control
// only appears on a runner that advertises `board_plan`, and it switches only
// the task list's sections. Every rule is `model/Plan.kt`'s; this draws, in
// Material's idiom: sentence case, tonal surfaces, and color only for a lane
// that's stale or waiting on you.

/** The Tasks | Plan control: a single-choice segmented row, as Material's own lists have it. */
@Composable
fun PlanSwitch(showsPlan: Boolean, onChange: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    SingleChoiceSegmentedButtonRow(modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag("plan-switch")) {
        listOf(false to "Tasks", true to "Plan").forEachIndexed { index, (plan, label) ->
            SegmentedButton(
                selected = showsPlan == plan,
                onClick = { onChange(plan) },
                shape = SegmentedButtonDefaults.itemShape(index, 2),
                modifier = Modifier.testTag("plan-switch-${label.lowercase()}"),
            ) { Text(label) }
        }
    }
}

/** The Plan view's rows, in a board's list. [state] is null before any read began. */
fun LazyListScope.planItems(
    state: PlanReadState?,
    statuses: Map<String, TaskStatus>,
    onOpen: (PlanPage) -> Unit,
    onRetry: () -> Unit,
    /** Which folds are open: Waiting for the next landing, Agents' finished, Landed today. */
    folds: PlanFolds = PlanFolds.Defaults,
    /** The workspace's Needs You count, which the band and the first section say. */
    needsYou: Int = 0,
    /** Where the Needs You row goes; null draws it without a way in. */
    onNeedsYou: (() -> Unit)? = null,
    /** Whether the runner lists agents (`board_agents`). */
    agentsSupported: Boolean = true,
    /** The themes grid's columns: one on a phone, two on a tablet. */
    themeColumns: Int = 1,
    /** The board's orchestrator pages (ov-285); null on a runner without `board_pages`. */
    pages: PagesHook? = null,
    /** Copy reference for rulings (ov-304); null on a runner without `board_rulings`. */
    rulings: RulingsHook? = null,
    /** Which of Now and Next the plan opens with; the peek leaves out what its card behind already says (ov-659). */
    band: BandShown = BandShown.ALL,
) {
    when (state) {
        null, PlanReadState.Loading -> item(key = "plan/loading") {
            Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) {
                CircularProgressIndicator(Modifier.testTag("plan-loading").semantics { contentDescription = "Reading the plan" })
            }
        }
        PlanReadState.NeedsUpdate -> item(key = "plan/needs-update") {
            PlanNotice(PlanWords.NEEDS_UPDATE, null, Modifier.testTag("plan-needs-update"))
        }
        PlanReadState.Reconnecting -> item(key = "plan/reconnecting") {
            // The runner isn't answering yet (ov-631): not a broken plan. The resync after the link reads again.
            Column(Modifier.testTag("plan-reconnecting")) {
                PlanNotice(PlanWords.RECONNECTING, null)
                TextButton(onClick = onRetry, contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp), modifier = Modifier.testTag("plan-retry")) {
                    Text(PlanWords.TRY_AGAIN)
                }
            }
        }
        PlanReadState.Unavailable -> item(key = "plan/unavailable") {
            Column(Modifier.testTag("plan-unavailable")) {
                PlanNotice(PlanWords.COULDNT_READ, null)
                TextButton(onClick = onRetry, contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp), modifier = Modifier.testTag("plan-retry")) {
                    Text(PlanWords.TRY_AGAIN)
                }
            }
        }
        is PlanReadState.Loaded -> {
            loaded(state.plan, statuses, onOpen, folds, needsYou, onNeedsYou, agentsSupported, themeColumns, pages, rulingsShown = rulings != null && state.plan.rulings.isNotEmpty(), band = band)
            // Last, as on the Mac and the iPhone: rulings ask nothing, and stand until the owner says otherwise.
            rulingItems(state.plan, rulings)
        }
    }
}

private fun LazyListScope.loaded(
    plan: Plan,
    statuses: Map<String, TaskStatus>,
    onOpen: (PlanPage) -> Unit,
    folds: PlanFolds,
    needsYou: Int,
    onNeedsYou: (() -> Unit)?,
    agentsSupported: Boolean,
    themeColumns: Int,
    pages: PagesHook?,
    rulingsShown: Boolean = false,
    band: BandShown = BandShown.ALL,
) {
    if (plan.isEmpty) {
        // Rulings alone plan nothing (review 1005a F1), but they're something to show.
        if (!rulingsShown) item(key = "plan/empty") {
            PlanNotice(PlanWords.NOTHING_PLANNED, PlanWords.NOTHING_PLANNED_DETAIL, Modifier.testTag("plan-empty"))
        }
        pages?.let { pageItems(it, plan, onOpen) }
        return
    }
    // One reading of the plan for the band and every section, so they can't disagree.
    val sections = PlanSections.of(plan, needsYou = needsYou, statuses = statuses, agentsSupported = agentsSupported)
    if (band != BandShown.NONE) item(key = "plan/band") { PlanGlanceBand(PlanGlanceSentence.of(sections), sections.needsYou, onNeedsYou = onNeedsYou, pill = false, shown = band) }
    planSections(plan, sections, statuses, folds, themeColumns, onOpen, onNeedsYou)
    // Behind `board_cost`: a runner without it sends no cost (ov-307).
    plan.cost?.takeIf { it.isWorthShowing }?.let { cost ->
        item(key = "plan/cost") { PlanHeader("Cost", null, Modifier.testTag("plan-cost-header")) }
        item(key = "plan/cost/block") { PlanCostBlock(cost) }
    }
    // After Themes, as on the Mac and the iPhone (design 6.1): pages of their
    // own, and anchored ones whose theme is gone.
    pages?.let { pageItems(it, plan, onOpen) }
    val landed = plan.landedToday()
    if (landed.isNotEmpty()) {
        val open = folds.isOpen(PlanFold.LANDED_TODAY)
        item(key = "plan/landed") {
            PlanHeader("Landed today", landed.size, Modifier.testTag("plan-landed-header"), open = open) { folds.toggle(PlanFold.LANDED_TODAY) }
        }
        if (open) {
            itemsIndexedLanes(landed, "landed") { _, lane ->
                PlanLaneRow(lane, plan.themeOf(lane), rank = null, now = plan.nowMs, waitsOnOwner = false) {
                    onOpen(PlanPage.Lane(lane.id))
                }
            }
        }
    }
}

private fun LazyListScope.itemsIndexedLanes(
    lanes: List<PlanLane>,
    group: String,
    row: @Composable (Int, PlanLane) -> Unit,
) {
    lanes.forEachIndexed { index, lane ->
        item(key = "plan/$group/${lane.id}") {
            row(index, lane)
        }
    }
}

/** A section's title and count, as the board's status headers read; a chevron when it opens. */
@Composable
fun PlanHeader(title: String, count: Int?, modifier: Modifier = Modifier, open: Boolean? = null, onClick: (() -> Unit)? = null) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = modifier
            .fillMaxWidth()
            .then(if (onClick != null) Modifier.clickable(role = androidx.compose.ui.semantics.Role.Button, onClick = onClick) else Modifier)
            .heightIn(min = 48.dp)
            .padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp)
            .semantics(mergeDescendants = true) {
                heading()
                contentDescription = if (count == null) title else "$title, $count"
                if (open != null) stateDescription = if (open) "Expanded" else "Collapsed"
            },
    ) {
        Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.weight(1f))
        // No count when it isn't known (a read that failed), never a zero.
        if (count != null) {
            Text(
                "$count",
                style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
                color = MaterialTheme.colorScheme.outline,
            )
        }
        if (open != null) {
            Spacer(Modifier.width(8.dp))
            Icon(
                if (open) Icons.Outlined.KeyboardArrowDown else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                contentDescription = null,
                modifier = Modifier.size(18.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/** "Nothing is planned on this board yet.", and why there would be. */
@Composable
fun PlanNotice(title: String, detail: String?, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(title, style = MaterialTheme.typography.titleSmall)
        if (detail != null) {
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** A lane state's glyph: neutral, a shape to scan by. */
fun planGlyph(state: LaneState): ImageVector = when (state) {
    LaneState.QUEUED -> Icons.Outlined.Schedule
    LaneState.BUILDING, LaneState.FIXING -> Icons.Outlined.Build
    LaneState.REVIEW -> Icons.Outlined.Visibility
    LaneState.LANDING -> Icons.Outlined.VerticalAlignBottom
    LaneState.LANDED -> Icons.Outlined.CheckCircleOutline
    LaneState.DROPPED -> Icons.Outlined.Close
    LaneState.UNKNOWN -> Icons.Outlined.QuestionMark
}

/**
 * One lane's row: its name and the theme it serves, then its reason or state.
 * In Next up its rank sits in the leading column; elsewhere its state's glyph
 * does, amber only when it's stale, waiting on you or over its token budget
 * (ov-307).
 */
@Composable
fun PlanLaneRow(
    lane: PlanLane,
    theme: PlanTheme?,
    rank: Int?,
    now: Long,
    waitsOnOwner: Boolean,
    /** Its main agent (ov-503): "Builder · Sonnet · 380K". */
    agent: PlanFleetAgent? = null,
    /** The heading of the landed train it still names, or null. */
    landedTrain: String? = null,
    onClick: () -> Unit,
) {
    val warning = if (waitsOnOwner) "Needs you" else PlanWords.stale(lane, now)
    val overBudget = PlanCostWords.overBudget(lane)
    val amber = glanceColor(GlancePalette.amber)
    val cards = if (lane.cards.isEmpty()) null else PlanWords.cards(lane.cards.size)
    val second = if (rank != null) lane.reason.ifEmpty { cards ?: "" }
    // The stage has a line of its own, so the state keeps only what it leaves to it ("Approved" beside "Checks failing", ov-317).
    else listOfNotNull(PlanWords.status(lane, stage = lane.shownStage == null), cards).joinToString(" · ")
    val stage = if (rank == null) lane.shownStage else null
    val agentWords = agent?.let { listOfNotNull(it.who, it.tokens()).joinToString(" · ") }
    val landedWords = landedTrain?.let { "Names $it, which landed" }
    val spoken = listOfNotNull(rank?.let { "${PlanWords.ordinal(it)} up" }, lane.heading, theme?.name, second, stage?.fullWords, agentWords, landedWords, warning, PlanCostWords.overBudgetSpoken(lane)).joinToString(", ")
    ListItem(
        leadingContent = {
            Box(Modifier.width(24.dp), contentAlignment = Alignment.CenterStart) {
                if (rank != null) {
                    Text("$rank", style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                } else {
                    Icon(planGlyph(lane.state), contentDescription = null, modifier = Modifier.size(20.dp), tint = if (warning == null && overBudget == null && stage?.needsAttention != true) MaterialTheme.colorScheme.onSurfaceVariant else amber)
                }
            }
        },
        // Which theme it serves (the owner's ask, ov-273) sits over the title: beside it, a long title is cut at a third.
        overlineContent = theme?.let {
            {
                Text(
                    it.name, maxLines = 1, overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.testTag("plan-lane-${lane.name}-theme"),
                )
            }
        },
        headlineContent = {
            Text(lane.heading, style = MaterialTheme.typography.titleSmall, maxLines = 3, overflow = TextOverflow.Ellipsis)
        },
        supportingContent = {
            Column {
                if (second.isNotEmpty()) Text(second, maxLines = 2, overflow = TextOverflow.Ellipsis)
                // Where its pull request stands (ov-317): gray, or amber for what needs a person, which leads with the problem.
                stage?.rowWords?.let {
                    Text(
                        it, maxLines = 1, overflow = TextOverflow.MiddleEllipsis,
                        style = MaterialTheme.typography.bodyMedium,
                        color = if (stage.needsAttention) amber else LocalContentColor.current,
                        modifier = Modifier.testTag("plan-lane-${lane.name}-stage${if (stage.needsAttention) "-attention" else ""}"),
                    )
                }
                if (agentWords != null) Text(agentWords, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-lane-${lane.name}-agent"))
                if (landedWords != null) Text(landedWords, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-lane-${lane.name}-landed"))
                if (warning != null) Text(warning, style = MaterialTheme.typography.labelMedium, color = amber)
                if (overBudget != null) Text(overBudget, style = MaterialTheme.typography.labelMedium, color = amber)
            }
        },
        trailingContent = {
            Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null, tint = MaterialTheme.colorScheme.outline)
        },
        modifier = Modifier.clickable(role = androidx.compose.ui.semantics.Role.Button, onClick = onClick).testTag("plan-lane-${lane.name}").semantics(mergeDescendants = true) { contentDescription = spoken },
    )
}

/**
 * A theme in the overview: its name and outcome, its progress, what's next
 * and, in amber, what needs you. Each theme reads on its own (the owner's
 * goal, ov-273).
 */
@Composable
fun PlanThemeRow(theme: PlanTheme, track: PlanTrack, now: Long, onClick: () -> Unit) {
    val spoken = listOfNotNull(
        theme.name, theme.outcome.ifEmpty { null }, PlanWords.progress(theme.counts), PlanWords.trackSpoken(track, now),
        theme.story.ifEmpty { null }, PlanWords.storyAge(theme, now),
        theme.next.ifEmpty { null }?.let { "Next: $it" }, theme.ownerAsk.ifEmpty { null }?.let { "Needs you: $it" },
    ).joinToString(". ")
    Row(
        verticalAlignment = Alignment.Top,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(role = androidx.compose.ui.semantics.Role.Button, onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 12.dp)
            .testTag("plan-theme-${theme.name}")
            .semantics(mergeDescendants = true) { contentDescription = spoken },
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(theme.name, style = MaterialTheme.typography.titleMedium, maxLines = 2)
                if (theme.state != "active") {
                    Spacer(Modifier.width(8.dp))
                    Text(theme.state.replaceFirstChar { it.uppercase() }, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            if (theme.outcome.isNotEmpty()) {
                Text(
                    theme.outcome,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = PlanWords.OUTCOME_LINES,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.testTag("plan-theme-outcome-${theme.name}"),
                )
            }
            PlanProgressBar(theme.counts, Modifier.padding(top = 4.dp))
            Text(
                PlanWords.progress(theme.counts),
                style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            // Moving or quiet, in words; amber only past a token budget (ov-307, ov-331), which says so here.
            PlanTrackLine(track, now)
            // Where it stands, three lines, and how old that account is.
            if (theme.story.isNotEmpty()) {
                Text(
                    theme.story,
                    style = MaterialTheme.typography.bodyMedium,
                    maxLines = PLAN_STORY_LINES,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(top = 4.dp).testTag("plan-theme-story-${theme.name}"),
                )
                PlanWords.storyAge(theme, now)?.let {
                    Text(it, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.outline)
                }
            }
            if (theme.next.isNotEmpty()) {
                Text("Next: ${theme.next}", style = MaterialTheme.typography.bodySmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
            if (theme.ownerAsk.isNotEmpty()) PlanAsk(theme.ownerAsk, maxLines = 2)
        }
        Icon(
            Icons.AutoMirrored.Outlined.KeyboardArrowRight,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.outline,
            modifier = Modifier.padding(start = 8.dp, top = 2.dp),
        )
    }
}

/** "mac-ux is in review", "Queued, 1st up", "No lane · quiet for 3 days": neutral, and amber only for a budget gone over. */
@Composable
fun PlanTrackLine(track: PlanTrack, now: Long, modifier: Modifier = Modifier) {
    val amber = glanceColor(GlancePalette.amber)
    val attention = track.needsAttention
    val color = if (attention) amber else MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        modifier.semantics(mergeDescendants = true) { contentDescription = PlanWords.trackSpoken(track, now) }
            .testTag(if (attention) "plan-budget-over" else "plan-theme-track"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(trackIcon(track), contentDescription = null, modifier = Modifier.size(16.dp), tint = color)
        Spacer(Modifier.width(6.dp))
        Text(
            PlanWords.track(track, now),
            style = MaterialTheme.typography.bodySmall.copy(fontWeight = if (attention) FontWeight.Medium else FontWeight.Normal),
            color = color,
        )
    }
}

/** A glyph per state, so the state never rests on color alone. */
private fun trackIcon(track: PlanTrack): ImageVector = when (track) {
    is PlanTrack.OverBudget, is PlanTrack.Stuck -> Icons.Outlined.ErrorOutline
    is PlanTrack.Moving -> Icons.Outlined.Sync
    is PlanTrack.Queued -> Icons.Outlined.Schedule
    is PlanTrack.Quiet -> Icons.Outlined.Bedtime
    PlanTrack.Idle -> Icons.Outlined.Circle
    PlanTrack.AllDone, PlanTrack.Done -> Icons.Outlined.CheckCircleOutline
    PlanTrack.Paused -> Icons.Outlined.PauseCircleOutline
}

/** "● Needs you: …": the one colored mark a theme has, with words. */
@Composable
fun PlanAsk(text: String, modifier: Modifier = Modifier, maxLines: Int = Int.MAX_VALUE) {
    val amber = glanceColor(GlancePalette.amber)
    Row(verticalAlignment = Alignment.Top, modifier = modifier.padding(top = 4.dp).testTag("plan-ask")) {
        Box(Modifier.padding(top = 6.dp).size(6.dp).clip(CircleShape).background(amber))
        Spacer(Modifier.width(6.dp))
        Text(
            androidx.compose.ui.text.buildAnnotatedString {
                pushStyle(androidx.compose.ui.text.SpanStyle(color = amber, fontWeight = androidx.compose.ui.text.font.FontWeight.Medium))
                append("Needs you: ")
                pop()
                append(text)
            },
            style = MaterialTheme.typography.bodySmall,
            maxLines = maxLines,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/**
 * A theme's cards by status, left to right, in neutral fills: done, in
 * review, in progress, not started. Canceled cards aren't drawn
 * ([PlanWords.segments]).
 */
@Composable
fun PlanProgressBar(counts: PlanCounts, modifier: Modifier = Modifier) {
    val parts = PlanWords.segments(counts)
    val total = PlanWords.total(counts).coerceAtLeast(1)
    val scheme = MaterialTheme.colorScheme
    Row(
        modifier.fillMaxWidth().height(4.dp)
            .clip(CircleShape).background(scheme.surfaceContainerHighest).semantics { contentDescription = PlanWords.breakdown(counts) },
        horizontalArrangement = Arrangement.spacedBy(1.dp),
    ) {
        for (part in parts) {
            Box(
                Modifier.weight(part.count.toFloat() / total).fillMaxHeight().background(
                    when (part.kind) {
                        PlanSegment.Kind.DONE -> scheme.onSurfaceVariant
                        PlanSegment.Kind.IN_REVIEW -> scheme.outline
                        PlanSegment.Kind.IN_PROGRESS -> scheme.outlineVariant
                        PlanSegment.Kind.NOT_STARTED -> scheme.surfaceContainerHighest
                    },
                ),
            )
        }
        val rest = total - parts.sumOf { it.count }
        if (rest > 0) Box(Modifier.weight(rest.toFloat() / total))
    }
}
