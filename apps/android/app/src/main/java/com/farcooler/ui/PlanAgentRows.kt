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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.PlanFleetAgent

// Agents (ov-503, design 6), drawn for the phone: one row per agent. Who (role and model) with its tokens beside it and
// its dollars after, then what it works for and its state. A working agent has a filled dot; a finished one, a ring.
// A row opens its lane when it has one.

@Composable
fun PlanAgentRow(agent: PlanFleetAgent, now: Long, onOpenLane: (String) -> Unit) {
    val lane = agent.laneId
    val tokens = agent.tokens()
    val cost = agent.cost()
    val second = agentSecondLine(agent, now)
    val spoken = listOfNotNull(agent.who, second, tokens?.let { "$it tokens" }, cost).joinToString(", ")
    val ink = MaterialTheme.colorScheme.onSurface
    Row(
        verticalAlignment = Alignment.Top,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
            .then(if (lane != null) Modifier.clickable(role = Role.Button) { onOpenLane(lane) } else Modifier)
            .padding(horizontal = 16.dp, vertical = 8.dp).testTag("plan-agent-${agent.id}")
            .semantics(mergeDescendants = true) { contentDescription = spoken },
    ) {
        Box(Modifier.padding(top = 7.dp).width(24.dp)) {
            Box(
                Modifier.size(8.dp).clip(CircleShape).then(
                    if (agent.working) Modifier.background(ink) else Modifier.border(1.5.dp, MaterialTheme.colorScheme.outline, CircleShape),
                ),
            )
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(agent.who, style = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.Medium), maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                // Columns that line up down the section: tokens, then dollars.
                Spacer(Modifier.width(8.dp))
                column(tokens, TOKENS_WIDTH)
                column(cost, COST_WIDTH)
            }
            Text(second, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
    }
}

private val TOKENS_WIDTH = 52.dp
private val COST_WIDTH = 72.dp

@Composable
private fun column(text: String?, width: androidx.compose.ui.unit.Dp) {
    Text(
        text ?: "", style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
        color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, textAlign = androidx.compose.ui.text.style.TextAlign.End,
        modifier = Modifier.width(width),
    )
}

/** "“Design ov-503” · Not on a lane · Done 1 h ago": what it was told, what it works for, and its state. */
internal fun agentSecondLine(agent: PlanFleetAgent, now: Long): String = buildList {
    if (agent.label.isNotEmpty()) add("“${agent.label}”")
    add(agent.worksFor)
    add(agent.state(now))
}.joinToString(" · ")
