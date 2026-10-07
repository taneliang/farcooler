package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.outlined.HelpOutline
import androidx.compose.material.icons.outlined.Stop
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.ExpandLess
import androidx.compose.material.icons.outlined.ExpandMore
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.AgentConversation
import com.farcooler.model.AgentRow
import kotlinx.coroutines.delay

/**
 * One row of the conversation view (ov-374), laid out for a phone's width: the
 * same rows as the Mac's and the iPhone's, from the runner's projector.
 *
 * A row is drawn from its own [AgentRow] and nothing else, so the list, keyed by
 * id, redraws only the rows whose object changed.
 */
@Composable
fun NativeRowView(row: AgentRow, showTerminal: () -> Unit) {
    Box(
        Modifier
            .fillMaxWidth()
            .alpha(if (row.provisional) 0.75f else 1f)
            .testTag("native-row-${row.id}"),
    ) {
        when (val kind = row.kind) {
            is AgentRow.Kind.OfTurn -> TurnRow(kind.turn)
            is AgentRow.Kind.OfProse -> MarkdownText(kind.prose.text, modifier = Modifier.fillMaxWidth())
            is AgentRow.Kind.OfThinking -> ThinkingRow(kind.thinking)
            is AgentRow.Kind.OfTool -> NativeToolRow(kind.tool)
            is AgentRow.Kind.OfSubagent -> SubagentRow(kind.subagent)
            is AgentRow.Kind.OfAsk -> AskRow(kind.ask, showTerminal)
            is AgentRow.Kind.OfQueued -> QueuedLine(kind.queued.text, kind.queued.state)
            is AgentRow.Kind.OfNotice -> NoticeLine(kind.notice.text)
            is AgentRow.Kind.OfHandoff -> HandoffRow(kind.handoff.reason, showTerminal)
            is AgentRow.Kind.OfGap -> NoticeLine(AgentConversation.gap(kind.gap))
            is AgentRow.Kind.Unknown -> Unit
        }
    }
}

/** The clock a running time counts against, ticking once a second while it's on screen. */
@Composable
private fun rememberNowMs(running: Boolean): Long {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(running) {
        while (running) {
            now = System.currentTimeMillis()
            delay(1000)
        }
    }
    return now
}

/** A run time: the finished span, or a timer counting from the start. */
@Composable
private fun RunTimeChip(startedMs: Long?, endedMs: Long?) {
    if (startedMs == null) return
    val now = rememberNowMs(running = endedMs == null)
    Text(
        AgentConversation.short((endedMs ?: now) - startedMs),
        style = MaterialTheme.typography.labelSmall,
        fontFamily = FontFamily.Monospace,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = 1,
    )
}

@Composable
private fun TurnRow(turn: AgentRow.Turn) {
    Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        // A turn whose prompt the projection never saw (a resume) has only its
        // outcome to show; one nobody typed is a notice, never the person's
        // message.
        if (AgentConversation.isNotice(turn) && turn.prompt.isNotEmpty()) {
            Text(
                AgentConversation.noticeText(turn),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth().testTag("native-notice-turn"),
            )
        } else if (turn.prompt.isNotEmpty()) {
            Bubble(turn.prompt, Modifier.testTag("native-prompt"))
        }
        // A notice's own turn says no time of its own: it's the line above, not
        // a message the person sent.
        if (!AgentConversation.isNotice(turn)) TurnStatus(turn)
    }
}

@Composable
private fun Bubble(text: String, modifier: Modifier = Modifier) {
    Row(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
        Spacer(Modifier.width(48.dp))
        Text(
            text,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier
                .clip(RoundedCornerShape(Radius.medium))
                .background(MaterialTheme.colorScheme.surfaceContainerHighest)
                .padding(horizontal = 12.dp, vertical = 8.dp),
        )
    }
}

@Composable
private fun TurnStatus(turn: AgentRow.Turn) {
    val failed = turn.outcome is AgentRow.Turn.Outcome.Failed
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        val style = MaterialTheme.typography.labelMedium
        val quiet = MaterialTheme.colorScheme.onSurfaceVariant
        if (turn.origin == "Queued") Text("From the queue", style = style, color = quiet)
        if (turn.backgroundRunning > 0) {
            Text(
                if (turn.backgroundRunning == 1) "1 agent still running" else "${turn.backgroundRunning} agents still running",
                style = style,
                color = quiet,
            )
        }
        val outcome = AgentConversation.outcome(turn)
        if (outcome != null) {
            // Color only where it needs attention: a failed turn.
            Text(outcome, style = style, color = if (failed) MaterialTheme.colorScheme.error else quiet)
        } else {
            CircularProgressIndicator(Modifier.size(12.dp), strokeWidth = 1.5.dp)
            RunTimeChip(turn.startedMs, null)
        }
    }
}

@Composable
private fun ThinkingRow(thinking: AgentRow.Thinking) {
    Row(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(
            if (thinking.endedMs == null) "Thinking" else "Thought for",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        RunTimeChip(thinking.startedMs, thinking.endedMs)
    }
}

/** A glyph for a tool's or a subagent's state. Color only where it needs attention: a failure. */
@Composable
private fun StatusMark(status: AgentRow.Status) {
    val quiet = MaterialTheme.colorScheme.onSurfaceVariant
    // Said to TalkBack: a failed tool is otherwise just its name and summary.
    val said = when (status) {
        AgentRow.Status.Running -> "Running"
        AgentRow.Status.Done -> "Done"
        AgentRow.Status.Failed -> "Failed"
        is AgentRow.Status.Ended -> "Stopped"
    }
    Box(Modifier.size(16.dp).semantics { stateDescription = said }, contentAlignment = Alignment.Center) {
        when (status) {
            AgentRow.Status.Running -> CircularProgressIndicator(Modifier.size(12.dp), strokeWidth = 1.5.dp)
            AgentRow.Status.Done -> Icon(Icons.Filled.Check, null, Modifier.size(14.dp), tint = quiet)
            AgentRow.Status.Failed -> Icon(Icons.Filled.Close, null, Modifier.size(14.dp), tint = MaterialTheme.colorScheme.error)
            is AgentRow.Status.Ended -> Icon(Icons.Outlined.Stop, null, Modifier.size(14.dp), tint = quiet)
        }
    }
}

@Composable
private fun NativeToolRow(tool: AgentRow.Tool) {
    var open by remember { mutableStateOf(false) }
    val hasDiff = tool.diff.isNotEmpty()
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(
            Modifier
                .fillMaxWidth()
                // Role.Button: TalkBack announces that it opens something.
                .then(if (hasDiff) Modifier.clickable(role = Role.Button) { open = !open } else Modifier)
                .padding(vertical = 2.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            StatusMark(tool.status)
            Text(tool.name, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
            Text(
                tool.summary,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
                modifier = Modifier.weight(1f),
            )
            if (hasDiff) {
                Icon(
                    if (open) Icons.Outlined.ExpandLess else Icons.Outlined.ExpandMore,
                    contentDescription = if (open) "Hide the diff" else "Show the diff",
                    modifier = Modifier.size(16.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            RunTimeChip(tool.startedMs, tool.endedMs)
        }
        if (open) NativeDiff(tool.diff)
    }
}

/** An edit's hunks, as unified-diff lines. */
@Composable
private fun NativeDiff(hunks: List<AgentRow.Hunk>) {
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Radius.small))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .horizontalScroll(rememberScrollState())
            .padding(8.dp),
    ) {
        for (hunk in hunks) {
            for (line in hunk.lines) {
                Text(
                    line.ifEmpty { " " },
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    softWrap = false,
                    // A removed line is quieter, an added one is the ink: no red and green.
                    color = if (line.startsWith("-")) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
                )
            }
        }
    }
}

@Composable
private fun SubagentRow(subagent: AgentRow.Subagent) {
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Radius.medium))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            StatusMark(subagent.status)
            Text(AgentConversation.agentType(subagent.agentType), style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium, modifier = Modifier.weight(1f))
            Text(
                if (subagent.toolCount == 1) "1 tool" else "${subagent.toolCount} tools",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            val running = subagent.status == AgentRow.Status.Running
            RunTimeChip(subagent.startedMs, if (running) null else (subagent.endedMs ?: subagent.lastMs))
        }
        Text(subagent.description, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
        if (subagent.status == AgentRow.Status.Running && subagent.currentAction.isNotEmpty()) {
            Text(
                subagent.currentAction,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/** A card that asks for attention: a ring in the tertiary color, only while something waits on the person. */
@Composable
private fun attentionCard(attention: Boolean): Modifier {
    val shape = RoundedCornerShape(Radius.medium)
    return Modifier
        .fillMaxWidth()
        .clip(shape)
        .background(MaterialTheme.colorScheme.surfaceContainerHigh)
        .then(if (attention) Modifier.border(1.dp, MaterialTheme.colorScheme.tertiary.copy(alpha = 0.55f), shape) else Modifier)
        .padding(12.dp)
}

@Composable
private fun AskRow(ask: AgentRow.Ask, showTerminal: () -> Unit) {
    Column(attentionCard(!ask.answered), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(if (ask.answered) Icons.Filled.Check else Icons.Outlined.HelpOutline, null, Modifier.size(18.dp))
            Text(AgentConversation.askTitle(ask), style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
        }
        Text(ask.text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 4, overflow = TextOverflow.Ellipsis)
        if (!ask.answered) OutlinedButton(onClick = showTerminal) { Text("Show terminal") }
    }
}

/** A message in claude's own queue (R-29), from its transcript or sent from here a moment ago. */
@Composable
fun QueuedLine(text: String, state: String = "Waiting") {
    Column(
        Modifier.fillMaxWidth().testTag("native-queued"),
        horizontalAlignment = Alignment.End,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Bubble(text)
        Text(
            AgentConversation.queuedLabel(state),
            style = MaterialTheme.typography.labelMedium,
            fontWeight = FontWeight.Medium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun NoticeLine(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        textAlign = TextAlign.Center,
        modifier = Modifier.fillMaxWidth(),
    )
}

/** Something only the terminal can show: a panel, a dialog, a question Claude is asking there. */
@Composable
fun HandoffRow(reason: String, showTerminal: () -> Unit) {
    Column(attentionCard(true).testTag("native-handoff"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(Icons.Outlined.Terminal, null, Modifier.size(18.dp))
            Text(reason, style = MaterialTheme.typography.bodyMedium)
        }
        OutlinedButton(onClick = showTerminal, modifier = Modifier.testTag("native-handoff-show-terminal")) { Text("Show terminal") }
    }
}
