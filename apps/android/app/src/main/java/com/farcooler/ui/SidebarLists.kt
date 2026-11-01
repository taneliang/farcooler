package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.Contrast
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.farcooler.model.OneTree
import com.farcooler.model.PlanCardGlyph
import com.farcooler.model.PlanPage
import com.farcooler.model.ThemeTree
import com.farcooler.model.TicketTree

// The sidebar as two lists (ov-503, design 3; ov-872), drawn for the phone and the tablet: Themes › Cards, the why, and
// Tickets › Agents, the how. `ThemeTree` and `TicketTree` decide what is in each; these lay them out. On a phone the tree's
// root has two rows that push one list each; on a tablet both lists stand in the list pane. A card and its ticket
// light each other: choose one and the other is tinted (`ThemeTree.ticketOf`, `TicketTree.cardOf`).

/** What was chosen last, which lights the other list's side of the join. */
sealed interface SidebarChoice {
    data class Card(val task: String) : SidebarChoice
    data class Ticket(val id: String) : SidebarChoice
}

/** How the lists reach the rest of the app. */
class SidebarActions(
    val onOpenTask: (String) -> Unit,
    val onOpenTicket: (String) -> Unit,
    val onOpenTheme: (String) -> Unit,
    /** An agent's pane, by its terminal id; null when no worktree holds it. */
    val onOpenTerminal: (String) -> Boolean,
)

private fun SidebarChoice?.isChosen(card: String? = null, ticket: String? = null): Boolean = when (this) {
    null -> false
    is SidebarChoice.Card -> card != null && task == card
    is SidebarChoice.Ticket -> ticket != null && id == ticket
}

/** The row's ground: the chosen row in the secondary container, the one it joins in a quieter tint. */
@Composable
private fun rowGround(chosen: Boolean, lit: Boolean): Color = when {
    chosen -> MaterialTheme.colorScheme.secondaryContainer
    lit -> MaterialTheme.colorScheme.surfaceContainerHigh
    else -> Color.Transparent
}

/** A list's title in the list pane: "Themes", with the filter beside it on a tablet. */
@Composable
internal fun SidebarListTitle(title: String, modifier: Modifier = Modifier) {
    Text(
        title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary,
        modifier = modifier.fillMaxWidth().heightIn(min = 40.dp).padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp)
            .semantics { heading() },
    )
}

/** The Themes list: each theme with its progress, and under it the cards the filter shows. */
fun LazyListScope.themeListItems(
    tree: ThemeTree,
    folds: PlanFolds,
    choice: SidebarChoice?,
    tickets: TicketTree,
    actions: SidebarActions,
    onChoose: (SidebarChoice) -> Unit,
) {
    if (tree.themes.isEmpty()) item(key = "themes/empty") {
        Text(
            if (tree.filter == OneTree.Filter.IN_REVIEW) "Nothing is in review." else "No cards yet. The orchestrator files them as it plans the work.",
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth().padding(16.dp).testTag("themes-empty"),
        )
    }
    for (theme in tree.themes) {
        // A theme with a ticket on the line opens; a quiet one starts folded.
        val open = folds.isOpen("theme:${theme.id}", default = theme.cards.any { it.ticketId != null })
        item(key = "theme/${theme.id}") {
            ThemeRow(theme, open, onToggle = { folds.toggle("theme:${theme.id}") }, onOpen = { actions.onOpenTheme(theme.id) })
        }
        if (open) for (card in theme.cards) item(key = "theme/${theme.id}/${card.task}") {
            CardRow(card, chosen = choice.isChosen(card = card.task), lit = choice is SidebarChoice.Ticket && tickets.cardOf(choice.id)?.task == card.task) {
                onChoose(SidebarChoice.Card(card.task))
                actions.onOpenTask(card.task)
            }
        }
    }
}

@Composable
private fun ThemeRow(theme: ThemeTree.ThemeNode, open: Boolean, onToggle: () -> Unit, onOpen: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onOpen).testTag("theme-row-${theme.name}")
            .semantics(mergeDescendants = false) { contentDescription = "${theme.name}, ${theme.progress}" },
    ) {
        Box(
            Modifier.size(48.dp).clickable(role = Role.Button, onClick = onToggle).testTag("theme-fold-${theme.name}")
                .semantics { stateDescription = if (open) "Expanded" else "Collapsed"; contentDescription = "${theme.name} cards" },
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                if (open) Icons.Outlined.KeyboardArrowDown else Icons.AutoMirrored.Outlined.KeyboardArrowRight, null,
                Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Text(theme.name, style = MaterialTheme.typography.titleSmall, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        Text(
            theme.progress, style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
            color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 8.dp, end = 16.dp),
        )
    }
}

@Composable
private fun CardRow(card: ThemeTree.CardNode, chosen: Boolean, lit: Boolean, onClick: () -> Unit) {
    val glyph = when (card.glyph) {
        PlanCardGlyph.DONE -> Icons.Outlined.CheckCircleOutline
        PlanCardGlyph.IN_PROGRESS -> Icons.Outlined.Contrast
        PlanCardGlyph.NOT_STARTED -> Icons.Outlined.RadioButtonUnchecked
    }
    val second = listOfNotNull(card.key.ifEmpty { null }, card.ticket).joinToString(" · ")
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().background(rowGround(chosen, lit)).heightIn(min = 48.dp)
            .clickable(role = Role.Button, onClick = onClick).padding(start = 48.dp, end = 16.dp, top = 6.dp, bottom = 6.dp)
            .testTag("card-row-${card.key.ifEmpty { card.task }}")
            .semantics(mergeDescendants = true) { contentDescription = listOfNotNull(card.title.ifEmpty { null }, second.ifEmpty { null }, if (lit) "On the ticket you chose" else null).joinToString(", ") },
    ) {
        Icon(glyph, null, Modifier.size(18.dp), tint = if (card.glyph == PlanCardGlyph.NOT_STARTED) MaterialTheme.colorScheme.outline else MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(
                card.title.ifEmpty { card.key }, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis,
                color = if (card.glyph == PlanCardGlyph.DONE) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
            )
            if (second.isNotEmpty()) Text(second, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

/**
 * The Tickets list: Landing first, with its landings inside it as "Landing N" and their tickets under them, then In
 * review, Building, and the folded Waiting; each ticket with its agents.
 */
fun LazyListScope.ticketListItems(
    tree: TicketTree,
    themes: ThemeTree,
    folds: PlanFolds,
    choice: SidebarChoice?,
    actions: SidebarActions,
    onChoose: (SidebarChoice) -> Unit,
) {
    if (tree.groups.isEmpty()) item(key = "tickets/empty") {
        Text(
            "No tickets yet.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth().padding(16.dp).testTag("tickets-empty"),
        )
    }
    for (group in tree.groups) {
        val id = "group:${group.kind}"
        val open = folds.isOpen(id, default = !group.folded)
        item(key = "ticket-group/$id") {
            GroupRow(group.title, group.detail, open, onToggle = { folds.toggle(id) })
        }
        if (open) for (landing in group.landings) {
            val landingId = "$id/landing:${landing.id}"
            item(key = "ticket-landing/$landingId") { LandingRow(landing) }
            for (ticket in landing.tickets) ticketItems(ticket, landingId, indent = 72.dp, themes, choice, actions, onChoose)
        }
        if (open) for (ticket in group.tickets) ticketItems(ticket, id, indent = 48.dp, themes, choice, actions, onChoose)
    }
}

private fun LazyListScope.ticketItems(
    ticket: TicketTree.TicketNode,
    under: String,
    indent: Dp,
    themes: ThemeTree,
    choice: SidebarChoice?,
    actions: SidebarActions,
    onChoose: (SidebarChoice) -> Unit,
) {
    item(key = "ticket/$under/${ticket.id}") {
        val lit = choice is SidebarChoice.Card && themes.ticketOf(choice.task) == ticket.id
        TicketRow(ticket, chosen = choice.isChosen(ticket = ticket.id), lit = lit, indent = indent) {
            onChoose(SidebarChoice.Ticket(ticket.id))
            actions.onOpenTicket(ticket.id)
        }
    }
    for (agent in ticket.agents) item(key = "ticket/$under/${ticket.id}/${agent.id}") { AgentRow(agent, actions, indent = indent + 24.dp) }
}

/** A landing inside Landing: "Landing 86", what it is for under it, and where it stands under that. */
@Composable
private fun LandingRow(landing: TicketTree.LandingNode) {
    Column(
        Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(start = 48.dp, end = 16.dp, top = 6.dp, bottom = 6.dp)
            .testTag("ticket-landing-${landing.label}")
            .semantics(mergeDescendants = true) { contentDescription = listOfNotNull(landing.label, landing.subtitle, landing.phase).joinToString(", ") },
    ) {
        Text(landing.label, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
        // Where it stands leads the second line, as on the Mac; the title follows and truncates first.
        Text(
            listOfNotNull(landing.phase, landing.subtitle).joinToString(" · "), style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis,
            modifier = Modifier.testTag("ticket-landing-${landing.label}-line"),
        )
    }
}

@Composable
private fun GroupRow(title: String, detail: String?, open: Boolean, onToggle: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onToggle).testTag("ticket-group-$title")
            .semantics(mergeDescendants = true) {
                contentDescription = listOfNotNull(title, detail).joinToString(", ")
                stateDescription = if (open) "Expanded" else "Collapsed"
            },
    ) {
        Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
            Icon(
                if (open) Icons.Outlined.KeyboardArrowDown else Icons.AutoMirrored.Outlined.KeyboardArrowRight, null,
                Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        // The count under a group's title, not beside it: at the tablet's 240 dp a long title would be cut to make room.
        Column(Modifier.weight(1f).padding(end = 16.dp, top = 4.dp, bottom = 4.dp)) {
            Text(title, style = MaterialTheme.typography.titleSmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (detail != null) Text(
                detail, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun TicketRow(ticket: TicketTree.TicketNode, chosen: Boolean, lit: Boolean, indent: Dp = 48.dp, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().background(rowGround(chosen, lit)).heightIn(min = 48.dp)
            .clickable(role = Role.Button, onClick = onClick).padding(start = indent, end = 16.dp, top = 6.dp, bottom = 6.dp)
            .testTag("ticket-row-${ticket.key}")
            .semantics(mergeDescendants = true) { contentDescription = listOfNotNull(ticket.heading, ticket.state, if (lit) "Is the card you chose" else null).joinToString(", ") },
    ) {
        Column(Modifier.weight(1f)) {
            Text(ticket.heading, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(
                listOf(ticket.key.takeIf { it != ticket.heading }, ticket.state.takeIf { it.isNotEmpty() }).filterNotNull().joinToString(" · "),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/** An agent under its ticket: role and model, a dot while it works, its tokens. Opens its pane when the runner read one. */
@Composable
private fun AgentRow(agent: TicketTree.AgentNode, actions: SidebarActions, indent: Dp = 72.dp) {
    val terminal = agent.terminal
    val opens = terminal != null
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().heightIn(min = if (opens) 48.dp else 32.dp)
            .then(if (terminal != null) Modifier.clickable(role = Role.Button) { actions.onOpenTerminal(terminal) } else Modifier)
            .padding(start = indent, end = 16.dp, top = 4.dp, bottom = 4.dp).testTag("agent-row-${agent.id}")
            .semantics(mergeDescendants = true) { contentDescription = listOfNotNull(agent.who, if (agent.working) "working" else "done", agent.tokens?.let { "$it tokens" }).joinToString(", ") },
    ) {
        Box(
            Modifier.size(7.dp).clip(CircleShape).then(
                if (agent.working) Modifier.background(MaterialTheme.colorScheme.onSurface) else Modifier.border(1.dp, MaterialTheme.colorScheme.outline, CircleShape),
            ),
        )
        Spacer(Modifier.width(12.dp))
        Text(
            agent.who, style = MaterialTheme.typography.bodySmall.copy(fontWeight = if (agent.working) FontWeight.Medium else FontWeight.Normal),
            color = if (agent.working) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
        )
        if (agent.tokens != null) Text(
            agent.tokens, style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
            color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 8.dp),
        )
    }
}
