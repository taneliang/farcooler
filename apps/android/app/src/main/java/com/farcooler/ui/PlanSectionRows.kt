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
import com.farcooler.model.PlanView
import com.farcooler.model.PlanViewKind
import com.farcooler.model.PlanViewTicket
import com.farcooler.model.ciOf
import com.farcooler.model.section
import com.farcooler.model.waiting
import com.farcooler.model.waitingCount

// The Plan's sections, drawn for the phone and the tablet (ov-503, design 7). The shared core's `PlanView` decides what is in
// each, in what order, and every word they say (ov-1016); these lay it out. Nothing here re-derives a rule.

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
    const val LANDED_TODAY = "landed-today"
    const val ON_LANDED_LANDING = "on-landed-landing"
}

/** What a section says with nothing in it: one quiet line. */
@Composable
fun PlanEmptyLine(words: String, modifier: Modifier = Modifier) {
    Text(
        words, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
    )
}

/** The sections [view] shows, in its order. */
fun LazyListScope.planSections(
    plan: Plan,
    view: PlanView,
    folds: PlanFolds,
    themeColumns: Int,
    onOpen: (PlanPage) -> Unit,
    onNeedsYou: (() -> Unit)?,
) {
    val now = view.now
    for (section in view.sections) {
        val id = "plan/${section.kind.name.lowercase()}"
        val count = section.count.takeIf { it > 0 }?.toInt()
        val empty = section.empty.takeIf { count == null }
        when (section.kind) {
            PlanViewKind.NeedsYou -> {
                item(key = "$id/header") { PlanHeader(section.title, section.count.toInt(), Modifier.testTag("plan-needs-you")) }
                item(key = "$id/row") { PlanNeedsYouRow(section.count.toInt(), onNeedsYou) }
            }
            PlanViewKind.Landing -> {
                item(key = "$id/header") {
                    // A long press says what a landing is, where landings first appear.
                    ExplainedOnLongPress(listOfNotNull(section.help)) { PlanHeader(section.title, count, Modifier.testTag("plan-landing")) }
                }
                if (empty != null) item(key = "$id/empty") { PlanEmptyLine(empty, Modifier.testTag("plan-landing-empty")) }
                for (card in view.landings) item(key = "plan/landing/${card.id}") {
                    PlanLandingCard(card, plan.landings.firstOrNull { it.id == card.id }?.let(plan::ciOf), now, onOpenTicket = { onOpen(PlanPage.Ticket(it)) }, help = section.help)
                }
            }
            PlanViewKind.InReview, PlanViewKind.Building -> {
                val tag = if (section.kind == PlanViewKind.InReview) "plan-in-review" else "plan-building"
                item(key = "$id/header") { PlanHeader(section.title, count, Modifier.testTag(tag)) }
                if (empty != null) item(key = "$id/empty") { PlanEmptyLine(empty, Modifier.testTag("$tag-empty")) }
                ticketRows(if (section.kind == PlanViewKind.InReview) view.inReview else view.building, id, onOpen)
            }
            PlanViewKind.Waiting -> {
                item(key = "$id/header") { PlanHeader(section.title, count, Modifier.testTag("plan-waiting")) }
                if (empty != null) item(key = "$id/empty") { PlanEmptyLine(empty, Modifier.testTag("plan-waiting-empty")) }
                // Each group is named only when another shares the section.
                for (group in view.waiting) {
                    val tag = "plan-" + group.kind.name.lowercase().replace('_', '-').let { if (it == "on-you") "waiting-on-you" else if (it == "next") "next-up" else it }
                    if (view.waiting.size > 1) item(key = "$id/$tag") { PlanGroupTitle(group.title, group.attention, Modifier.testTag(tag)) }
                    ticketRows(group.cards, "$id/$tag", onOpen)
                }
            }
            PlanViewKind.OnLandedLanding -> {
                val open = folds.isOpen(PlanFold.ON_LANDED_LANDING, default = !section.folded)
                item(key = "$id/header") {
                    PlanHeader(section.title, count, Modifier.testTag("plan-on-landed-landing"), open = open) { folds.toggle(PlanFold.ON_LANDED_LANDING) }
                }
                if (open) ticketRows(view.onLandedLanding, id, onOpen)
            }
            PlanViewKind.Agents -> {
                item(key = "$id/header") { PlanHeader(section.title, count, Modifier.testTag("plan-agents")) }
                if (view.agents.rows.isEmpty()) {
                    item(key = "$id/empty") { PlanEmptyLine(view.agents.header, Modifier.testTag("plan-agents-empty")) }
                } else {
                    item(key = "$id/summary") {
                        Text(
                            view.agents.header, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 16.dp).testTag("plan-agents-header"),
                        )
                    }
                    for (agent in view.agents.rows) item(key = "$id/${agent.id}") {
                        PlanAgentRow(agent, plan.agents.firstOrNull { it.id == agent.id }?.tickets?.firstOrNull()?.task) { onOpen(PlanPage.Ticket(it)) }
                    }
                }
            }
            PlanViewKind.Themes -> {
                item(key = "$id/header") { PlanHeader(section.title, section.count.toInt(), Modifier.testTag("plan-themes")) }
                themeGrid(
                    view.themes, themeColumns,
                    budget = { themeId -> plan.themes.firstOrNull { it.id == themeId }?.let(PlanCostWords::overBudget)?.let { PlanCostWords.budgetLine(it) } },
                    onOpen = { onOpen(PlanPage.Theme(it)) },
                )
            }
            // Drawn last, after Cost and Pages (`PlanScreens`).
            PlanViewKind.LandedToday -> {}
        }
    }
}

internal fun LazyListScope.ticketRows(cards: List<PlanViewTicket>, group: String, onOpen: (PlanPage) -> Unit) {
    for (card in cards) item(key = "$group/${card.id}") { PlanCardRow(card) { onOpen(PlanPage.Ticket(card.id)) } }
}

/** A quiet subtitle inside a section: "Next up", "Not ranked". */
@Composable
private fun PlanGroupTitle(title: String, attention: Boolean, modifier: Modifier = Modifier) {
    Text(
        title, style = MaterialTheme.typography.labelLarge, color = if (attention) glanceColor(GlancePalette.amber) else MaterialTheme.colorScheme.onSurfaceVariant,
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
