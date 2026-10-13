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
import androidx.compose.ui.unit.dp
import com.farcooler.model.LaneTree
import com.farcooler.model.OneTree
import com.farcooler.model.PlanCardGlyph
import com.farcooler.model.PlanPage
import com.farcooler.model.ThemeTree

// The sidebar as two lists (ov-503, design 3), drawn for the phone and the tablet: Themes › Cards, the why, and
// Lanes › Agents, the how. `ThemeTree` and `LaneTree` decide what is in each; these lay them out. On a phone the tree's
// root has two rows that push one list each; on a tablet both lists stand in the list pane. A card and the lane working
// it light each other: choose one and the other is tinted (`ThemeTree.laneOf`, `LaneTree.cardsOf`).

/** What was chosen last, which lights the other list's side of the join. */
sealed interface SidebarChoice {
    data class Card(val task: String) : SidebarChoice
    data class Lane(val id: String) : SidebarChoice
}

/** How the lists reach the rest of the app. */
class SidebarActions(
    val onOpenTask: (String) -> Unit,
    val onOpenLane: (String) -> Unit,
    val onOpenTheme: (String) -> Unit,
    /** An agent's pane, by its terminal id; null when no worktree holds it. */
    val onOpenTerminal: (String) -> Boolean,
)

private fun SidebarChoice?.isChosen(card: String? = null, lane: String? = null): Boolean = when (this) {
    null -> false
    is SidebarChoice.Card -> card != null && task == card
    is SidebarChoice.Lane -> lane != null && id == lane
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
    lanes: LaneTree,
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
        // A theme a lane works opens; a quiet one starts folded.
        val open = folds.isOpen("theme:${theme.id}", default = theme.cards.any { it.laneId != null })
        item(key = "theme/${theme.id}") {
            ThemeRow(theme, open, onToggle = { folds.toggle("theme:${theme.id}") }, onOpen = { actions.onOpenTheme(theme.id) })
        }
        if (open) for (card in theme.cards) item(key = "theme/${theme.id}/${card.task}") {
            CardRow(card, chosen = choice.isChosen(card = card.task), lit = choice is SidebarChoice.Lane && lanes.cardsOf(choice.id).any { it.task == card.task }) {
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
    val second = listOfNotNull(card.key.ifEmpty { null }, card.lane).joinToString(" · ")
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().background(rowGround(chosen, lit)).heightIn(min = 48.dp)
            .clickable(role = Role.Button, onClick = onClick).padding(start = 48.dp, end = 16.dp, top = 6.dp, bottom = 6.dp)
            .testTag("card-row-${card.key.ifEmpty { card.task }}")
            .semantics(mergeDescendants = true) { contentDescription = listOfNotNull(card.title.ifEmpty { null }, second.ifEmpty { null }, if (lit) "Shares a lane" else null).joinToString(", ") },
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

/** The Lanes list: trains first, then In review, Building, and the folded Waiting; each lane with its agents. */
fun LazyListScope.laneListItems(
    tree: LaneTree,
    themes: ThemeTree,
    folds: PlanFolds,
    choice: SidebarChoice?,
    actions: SidebarActions,
    onChoose: (SidebarChoice) -> Unit,
) {
    if (tree.groups.isEmpty()) item(key = "lanes/empty") {
        Text(
            "No lanes yet.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth().padding(16.dp).testTag("lanes-empty"),
        )
    }
    for (group in tree.groups) {
        val id = "group:${group.kind}"
        val open = folds.isOpen(id, default = !group.folded)
        item(key = "lane-group/$id") {
            GroupRow(group.title, group.detail, open, onToggle = { folds.toggle(id) })
        }
        if (open) for (lane in group.lanes) {
            item(key = "lane/$id/${lane.id}") {
                val lit = choice is SidebarChoice.Card && themes.laneOf(choice.task) == lane.id
                LaneRow(lane, chosen = choice.isChosen(lane = lane.id), lit = lit) {
                    onChoose(SidebarChoice.Lane(lane.id))
                    actions.onOpenLane(lane.id)
                }
            }
            for (agent in lane.agents) item(key = "lane/$id/${lane.id}/${agent.id}") { AgentRow(agent, actions) }
        }
    }
}

@Composable
private fun GroupRow(title: String, detail: String?, open: Boolean, onToggle: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onToggle).testTag("lane-group-$title")
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
        // The phase under a train's title, not beside it: at the tablet's 240 dp a long title would be cut to make room.
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
private fun LaneRow(lane: LaneTree.LaneNode, chosen: Boolean, lit: Boolean, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().background(rowGround(chosen, lit)).heightIn(min = 48.dp)
            .clickable(role = Role.Button, onClick = onClick).padding(start = 48.dp, end = 16.dp, top = 6.dp, bottom = 6.dp)
            .testTag("lane-row-${lane.name}")
            .semantics(mergeDescendants = true) { contentDescription = listOfNotNull(lane.heading, lane.state, if (lit) "Works the card you chose" else null).joinToString(", ") },
    ) {
        Column(Modifier.weight(1f)) {
            Text(lane.heading, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(
                listOf(lane.name.takeIf { it != lane.heading }, lane.state).filterNotNull().joinToString(" · "),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/** An agent under its lane: role and model, a dot while it works, its tokens. Opens its pane when the runner read one. */
@Composable
private fun AgentRow(agent: LaneTree.AgentNode, actions: SidebarActions) {
    val terminal = agent.terminal
    val opens = terminal != null
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().heightIn(min = if (opens) 48.dp else 32.dp)
            .then(if (terminal != null) Modifier.clickable(role = Role.Button) { actions.onOpenTerminal(terminal) } else Modifier)
            .padding(start = 72.dp, end = 16.dp, top = 4.dp, bottom = 4.dp).testTag("agent-row-${agent.id}")
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
