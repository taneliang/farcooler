package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Flag
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.farcooler.model.GlancePalette
import com.farcooler.model.Plan
import com.farcooler.model.PlanCostWords
import com.farcooler.model.PlanPage
import com.farcooler.model.PlanSections
import com.farcooler.model.ciOf
import com.farcooler.model.themeGroups

// The Plan's sections, drawn for the phone and the tablet (ov-503, design 7). `PlanSections` decides what is in each and
// what it says empty; these lay it out, in its order: Needs you, Shipping, In review, Building, Waiting, On a landed
// train (folded), Agents, Themes. Nothing here re-derives a rule.

/** Which of the plan's folds are open: a fold's own default, flipped by a tap, kept while the screen is. */
class PlanFolds(private val flipped: List<String>, private val flip: (String) -> Unit) {
    /** Whether the fold [id] shows its rows; [default] is how it starts. */
    fun isOpen(id: String, default: Boolean = false): Boolean = default != (id in flipped)

    fun toggle(id: String) = flip(id)

    companion object {
        /** All at their defaults, and taps ignored: for a picture of the plan as it opens. */
        val Defaults = PlanFolds(emptyList()) {}
    }
}

@Composable
fun rememberPlanFolds(key: Any?): PlanFolds {
    val flipped = rememberSaveable(key) { mutableStateListOf<String>() }
    return PlanFolds(flipped) { id -> if (id in flipped) flipped.remove(id) else flipped.add(id) }
}

internal object PlanFold {
    const val LANDED_TRAIN = "landed-train"
    const val AGENTS_FINISHED = "agents-finished"
    const val LANDED_TODAY = "landed-today"
}

/** What a section says with nothing in it: one quiet line. */
@Composable
fun PlanEmptyLine(words: String, modifier: Modifier = Modifier) {
    Text(
        words, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
    )
}

/** The sections [sections] shows, in its order. */
fun LazyListScope.planSections(
    plan: Plan,
    sections: PlanSections,
    statuses: Map<String, com.farcooler.model.TaskStatus>,
    folds: PlanFolds,
    themeColumns: Int,
    onOpen: (PlanPage) -> Unit,
    onNeedsYou: (() -> Unit)?,
) {
    val now = sections.now
    for (kind in sections.shown) {
        val id = "plan/${kind.name.lowercase()}"
        when (kind) {
            PlanSections.Kind.NEEDS_YOU -> {
                item(key = "$id/header") { PlanHeader(kind.title, sections.needsYou, Modifier.testTag("plan-needs-you")) }
                item(key = "$id/row") { PlanNeedsYouRow(sections.needsYou, onNeedsYou) }
            }
            PlanSections.Kind.SHIPPING -> {
                item(key = "$id/header") { PlanHeader(kind.title, sections.count(kind).takeIf { it > 0 }, Modifier.testTag("plan-shipping")) }
                val empty = sections.emptyWords(kind)
                if (empty != null) item(key = "$id/empty") { PlanEmptyLine(empty, Modifier.testTag("plan-shipping-empty")) }
                for (card in sections.shipping) item(key = "plan/train/${card.id}") {
                    PlanTrainCard(card, plan.ciOf(card.train), now, onOpenLane = { onOpen(PlanPage.Lane(it)) })
                }
            }
            PlanSections.Kind.IN_REVIEW, PlanSections.Kind.BUILDING -> {
                val tag = if (kind == PlanSections.Kind.IN_REVIEW) "plan-in-review" else "plan-building"
                val rows = if (kind == PlanSections.Kind.IN_REVIEW) sections.inReview else sections.building
                item(key = "$id/header") { PlanHeader(kind.title, rows.size.takeIf { it > 0 }, Modifier.testTag(tag)) }
                val empty = sections.emptyWords(kind)
                if (empty != null) item(key = "$id/empty") { PlanEmptyLine(empty, Modifier.testTag("$tag-empty")) }
                laneRows(plan, rows, id, now, rank = false, onOpen)
            }
            PlanSections.Kind.WAITING -> {
                item(key = "$id/header") { PlanHeader(kind.title, sections.count(kind).takeIf { it > 0 }, Modifier.testTag("plan-waiting")) }
                val empty = sections.emptyWords(kind)
                if (empty != null) item(key = "$id/empty") { PlanEmptyLine(empty, Modifier.testTag("plan-waiting-empty")) }
                val waiting = sections.waiting
                // Next up ranked, then Not ranked, then Waiting on you, each named only when something else shares the section.
                val groups = listOf(
                    Triple("Next up", waiting.next, "plan-next-up"),
                    Triple("Not ranked", waiting.notRanked, "plan-not-ranked"),
                    Triple("Waiting on you", waiting.onYou, "plan-waiting-on-you"),
                ).filter { it.second.isNotEmpty() }
                for ((title, rows, tag) in groups) {
                    if (groups.size > 1) item(key = "$id/$tag") { PlanGroupTitle(title, Modifier.testTag(tag)) }
                    laneRows(plan, rows, "$id/$tag", now, rank = rows === waiting.next, onOpen)
                }
            }
            PlanSections.Kind.ON_LANDED_TRAIN -> {
                val open = folds.isOpen(PlanFold.LANDED_TRAIN)
                item(key = "$id/header") {
                    PlanHeader(kind.title, sections.onLandedTrain.size, Modifier.testTag("plan-on-landed-train"), open = open) { folds.toggle(PlanFold.LANDED_TRAIN) }
                }
                if (open) laneRows(plan, sections.onLandedTrain, id, now, rank = false, onOpen)
            }
            PlanSections.Kind.AGENTS -> {
                val fleet = sections.fleet
                item(key = "$id/header") { PlanHeader(kind.title, sections.count(kind).takeIf { it > 0 }, Modifier.testTag("plan-agents")) }
                if (fleet.isEmpty) {
                    item(key = "$id/empty") { PlanEmptyLine(PlanSections.Kind.AGENTS.emptyWords ?: "", Modifier.testTag("plan-agents-empty")) }
                } else {
                    item(key = "$id/summary") {
                        Text(
                            fleet.header(), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 16.dp).testTag("plan-agents-header"),
                        )
                    }
                    for (agent in fleet.working) item(key = "$id/${agent.id}") { PlanAgentRow(agent, now) { onOpen(PlanPage.Lane(it)) } }
                    if (fleet.finished.isNotEmpty()) {
                        val open = folds.isOpen(PlanFold.AGENTS_FINISHED, default = fleet.working.isEmpty())
                        item(key = "$id/finished") {
                            PlanHeader("Finished", fleet.finished.size, Modifier.testTag("plan-agents-finished"), open = open) { folds.toggle(PlanFold.AGENTS_FINISHED) }
                        }
                        if (open) for (agent in fleet.finished) item(key = "$id/${agent.id}") { PlanAgentRow(agent, now) { onOpen(PlanPage.Lane(it)) } }
                    }
                }
            }
            PlanSections.Kind.THEMES -> {
                item(key = "$id/header") { PlanHeader(kind.title, sections.count(kind), Modifier.testTag("plan-themes")) }
                themeGrid(
                    sections.themes, themeColumns,
                    budget = { themeId -> plan.themes.firstOrNull { it.id == themeId }?.let(PlanCostWords::overBudget)?.let { PlanCostWords.budgetLine(it) } },
                    onOpen = { onOpen(PlanPage.Theme(it)) },
                )
            }
        }
    }
}

private fun LazyListScope.laneRows(
    plan: Plan,
    rows: List<PlanSections.LaneRow>,
    group: String,
    now: Long,
    rank: Boolean,
    onOpen: (PlanPage) -> Unit,
) {
    rows.forEachIndexed { index, row ->
        item(key = "$group/${row.id}") {
            PlanLaneRow(
                row.lane, plan.themeOf(row.lane), rank = if (rank) index + 1 else null, now = now, waitsOnOwner = row.waitsOnOwner,
                agent = row.agent, landedTrain = row.landedTrain,
            ) { onOpen(PlanPage.Lane(row.id)) }
        }
    }
}

/** A quiet subtitle inside a section: "Next up", "Not ranked". */
@Composable
private fun PlanGroupTitle(title: String, modifier: Modifier = Modifier) {
    Text(
        title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 2.dp),
    )
}

/** The way to what needs you: the flag carries the color, the words stay the text's own. */
@Composable
private fun PlanNeedsYouRow(count: Int, onClick: (() -> Unit)?) {
    ListItem(
        leadingContent = { Icon(Icons.Outlined.Flag, null, Modifier.size(20.dp), tint = glanceColor(GlancePalette.amber)) },
        headlineContent = { Text("Decisions, questions and reviews") },
        supportingContent = { Text(needsYouWords(count)) },
        trailingContent = if (onClick != null) ({ Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.outline) }) else null,
        modifier = (if (onClick != null) Modifier.clickable(role = Role.Button, onClick = onClick) else Modifier).testTag("plan-needs-you-row"),
    )
}
