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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.GlancePalette
import com.farcooler.model.Plan
import com.farcooler.model.PlanCostWords
import com.farcooler.model.PlanCounts
import com.farcooler.model.PlanGlanceSentence
import com.farcooler.model.PlanView
import com.farcooler.model.PlanViewMark
import com.farcooler.model.PlanViewNote
import com.farcooler.model.PlanViewKind
import com.farcooler.model.PlanViewNoteKind
import com.farcooler.model.PlanViewTicket
import com.farcooler.model.PlanViews
import com.farcooler.model.PlanTicket
import com.farcooler.model.TicketStage
import com.farcooler.model.ciOf
import com.farcooler.model.PlanPage
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanSegment
import com.farcooler.model.PlanTheme
import com.farcooler.model.PlanWords
import com.farcooler.model.TaskStatus

// The Plan view on Android (ov-268 design 6.5): what the board's list shows
// when Plan is chosen. Next up first, because it's what the owner can't see
// anywhere else; then the tickets being worked now, the themes, and what landed
// today. A ticket or a theme opens its page, pushed.
//
// EXPERIMENTAL and opt-in, as the layer is: Tasks is the default, the control
// only appears on a runner that advertises `board_plan`, and it switches only
// the task list's sections. Every rule is `model/Plan.kt`'s; this draws, in
// Material's idiom: sentence case, tonal surfaces, and color only for a ticket
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
    /** Which folds are open: Landed today. */
    folds: PlanFolds = PlanFolds.Defaults,
    /** The workspace's Needs You count, which the band and the first section say. */
    needsYou: Int = 0,
    /** Where the Needs You row goes; null draws it without a way in. */
    onNeedsYou: (() -> Unit)? = null,
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
            loaded(state.plan, statuses, onOpen, folds, needsYou, onNeedsYou, themeColumns, pages, rulingsShown = rulings != null && state.plan.rulings.isNotEmpty(), band = band)
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
    // One reading of the plan for the band and every section, so they can't disagree: the shared core's (ov-1016).
    val view = PlanViews.of(plan, needsYou = needsYou, statuses = statuses)
    if (view == null) {
        item(key = "plan/unreadable") { PlanNotice(PlanWords.COULDNT_READ, null, Modifier.testTag("plan-unreadable")) }
    } else {
        if (band != BandShown.NONE) item(key = "plan/band") { PlanGlanceBand(PlanGlanceSentence.of(view), view.needsYou, onNeedsYou = onNeedsYou, pill = false, shown = band) }
        planSections(plan, view, folds, themeColumns, onOpen, onNeedsYou)
    }
    // Behind `board_cost`: a runner without it sends no cost (ov-307).
    plan.cost?.takeIf { it.isWorthShowing }?.let { cost ->
        item(key = "plan/cost") { PlanHeader("Cost", null, Modifier.testTag("plan-cost-header")) }
        item(key = "plan/cost/block") { PlanCostBlock(cost) }
    }
    // After Themes, as on the Mac and the iPhone (design 6.1): pages of their
    // own, and anchored ones whose theme is gone.
    pages?.let { pageItems(it, plan, onOpen) }
    view?.section(PlanViewKind.LANDED_TODAY)?.let { landed ->
        val open = folds.isOpen(PlanFold.LANDED_TODAY, default = !landed.folded)
        item(key = "plan/landed") {
            PlanHeader(landed.title, landed.count, Modifier.testTag("plan-landed-header"), open = open) { folds.toggle(PlanFold.LANDED_TODAY) }
        }
        if (open) ticketRows(view.landedToday, "plan/landed", onOpen)
    }
}

/** A section's title and count, as the board's status headers read; a chevron when it opens. */
@Composable
fun PlanHeader(
    title: String,
    count: Int?,
    modifier: Modifier = Modifier,
    open: Boolean? = null,
    onClick: (() -> Unit)? = null,
    // A button drawn before the count, as Decided for you's Keep all (ov-637):
    // before it, so the count isn't read as the button's. A button of its own,
    // not merged into the heading.
    action: (@Composable () -> Unit)? = null,
) {
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
        if (action != null) {
            action()
            Spacer(Modifier.width(8.dp))
        }
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

/** A ticket stage's glyph: neutral, a shape to scan by. */
fun planGlyph(stage: TicketStage): ImageVector = when (stage) {
    TicketStage.WAITING -> Icons.Outlined.Schedule
    TicketStage.BUILDING -> Icons.Outlined.Build
    TicketStage.REVIEWING -> Icons.Outlined.Visibility
    TicketStage.NEEDS_YOU -> Icons.Outlined.ErrorOutline
    TicketStage.LANDING -> Icons.Outlined.VerticalAlignBottom
    TicketStage.LANDED -> Icons.Outlined.CheckCircleOutline
    TicketStage.DROPPED -> Icons.Outlined.Close
    TicketStage.UNKNOWN -> Icons.Outlined.QuestionMark
}

/** A card's mark: its place in Next Up in numerals, else a glyph per stage. */
private fun markGlyph(mark: PlanViewMark): ImageVector = when (mark) {
    PlanViewMark.WAITING -> Icons.Outlined.Schedule
    PlanViewMark.BUILDING, PlanViewMark.FIXING -> Icons.Outlined.Build
    PlanViewMark.REVIEWING -> Icons.Outlined.Visibility
    PlanViewMark.NEEDS_YOU -> Icons.Outlined.ErrorOutline
    PlanViewMark.LANDING -> Icons.Outlined.VerticalAlignBottom
    PlanViewMark.LANDED -> Icons.Outlined.CheckCircleOutline
    PlanViewMark.DROPPED -> Icons.Outlined.Close
    PlanViewMark.RANK, PlanViewMark.UNKNOWN -> Icons.Outlined.QuestionMark
}

/**
 * One ticket's row, drawn from the core's card (ov-1016): the theme it serves over the title, then the card's second
 * line and its notes. In Next up its rank sits in the leading column; elsewhere its stage's glyph does, amber when a
 * note needs a person. A note with an explanation says it on a long press. Every word is the card's.
 */
@Composable
fun PlanCardRow(card: PlanViewTicket, onClick: () -> Unit) {
    val amber = glanceColor(GlancePalette.amber)
    // The agent keeps the line of its own it has always had, smaller, under the second line.
    val agentWords = card.agent?.line?.ifEmpty { null }
    val second = card.second
    ListItem(
        leadingContent = {
            Box(Modifier.width(24.dp), contentAlignment = Alignment.CenterStart) {
                if (card.rank != null) {
                    Text("${card.rank}", style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                } else {
                    Icon(markGlyph(card.mark), contentDescription = null, modifier = Modifier.size(20.dp), tint = if (card.markAttention) amber else MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        },
        // Which theme it serves (the owner's ask, ov-273) sits over the title: beside it, a long title is cut at a third.
        overlineContent = card.theme?.let {
            {
                Text(it, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-ticket-${card.key}-theme"))
            }
        },
        headlineContent = {
            Text(card.heading, style = MaterialTheme.typography.titleSmall, maxLines = 3, overflow = TextOverflow.Ellipsis)
        },
        supportingContent = {
            Column {
                if (second.isNotEmpty()) Text(second, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-ticket-${card.key}-second"))
                if (agentWords != null) Text(agentWords, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-ticket-${card.key}-agent"))
                card.notes.forEach { PlanCardNote(card.key, it) }
            }
        },
        trailingContent = {
            Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null, tint = MaterialTheme.colorScheme.outline)
        },
        modifier = Modifier.clickable(role = androidx.compose.ui.semantics.Role.Button, onClick = onClick).testTag("plan-ticket-${card.key}")
            .semantics(mergeDescendants = true) { contentDescription = card.accessibility },
    )
}

/** A line under a card's second line: where its pull request stands, the engine's step, a warning, a budget gone over. */
@Composable
private fun PlanCardNote(key: String, note: PlanViewNote) {
    val amber = glanceColor(GlancePalette.amber)
    val line: @Composable () -> Unit = {
        when (note.kind) {
            // Where its pull request stands (ov-317): gray, or amber for what needs a person, which leads with the problem.
            PlanViewNoteKind.STAGE -> Text(
                note.text, maxLines = 1, overflow = TextOverflow.MiddleEllipsis, style = MaterialTheme.typography.bodyMedium,
                color = if (note.attention) amber else LocalContentColor.current,
                modifier = Modifier.testTag("plan-ticket-$key-stage${if (note.attention) "-attention" else ""}"),
            )
            PlanViewNoteKind.RUN -> Text(
                note.text, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall,
                color = if (note.attention) amber else MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.testTag("plan-ticket-$key-run"),
            )
            else -> Text(note.text, style = MaterialTheme.typography.labelMedium, color = if (note.attention) amber else LocalContentColor.current)
        }
    }
    if (note.help.isNullOrEmpty()) line() else ExplainedOnLongPress(listOf(note.help)) { line() }
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
