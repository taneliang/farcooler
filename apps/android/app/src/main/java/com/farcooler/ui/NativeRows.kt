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
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material.icons.outlined.Stop
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.ExpandLess
import androidx.compose.material.icons.outlined.ExpandMore
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
fun NativeRowView(row: AgentRow, showTerminal: () -> Unit, answer: NativeAnswer? = null, sendNow: (() -> Unit)? = null) {
    Box(
        Modifier
            .fillMaxWidth()
            .alpha(if (row.provisional) 0.75f else 1f)
            .testTag("native-row-${row.id}"),
    ) {
        when (val kind = row.kind) {
            is AgentRow.Kind.OfTurn -> TurnRow(row.id, kind.turn)
            is AgentRow.Kind.OfProse -> MarkdownText(kind.prose.text, modifier = Modifier.fillMaxWidth())
            is AgentRow.Kind.OfThinking -> ThinkingRow(kind.thinking)
            is AgentRow.Kind.OfTool -> NativeToolRow(row.id, kind.tool)
            is AgentRow.Kind.OfSubagent -> SubagentRow(kind.subagent)
            is AgentRow.Kind.OfAsk -> NativeAskRow(kind.ask, answer, showTerminal)
            is AgentRow.Kind.OfQueued -> QueuedLine(kind.queued.text, kind.queued.state, sendNow)
            is AgentRow.Kind.OfNotice -> NoticeLine(kind.notice.text)
            is AgentRow.Kind.OfHandoff -> HandoffRow(kind.handoff.reason, showTerminal)
            is AgentRow.Kind.OfGap -> NoticeLine(AgentConversation.gap(kind.gap))
            is AgentRow.Kind.OfTasks -> NativeTasksRow(kind.tasks)
            is AgentRow.Kind.OfHint, is AgentRow.Kind.Unknown -> Unit
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
internal fun RunTimeChip(startedMs: Long?, endedMs: Long?) {
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
private fun TurnRow(row: String, turn: AgentRow.Turn) {
    val images = LocalPromptImages.current
    // A thumbnail drawn stands for its `[Image #N]`, which leaves the words
    // (ov-454); an image with none drawn keeps its placeholder.
    val prompt = if (images?.source != null) AgentConversation.withoutPlaceholders(turn.prompt, turn.images.size) else turn.prompt
    Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (images != null && images.source != null && turn.images.isNotEmpty()) PromptImageStrip(row, turn.images, images)
        // A turn whose prompt the projection never saw (a resume) has only its
        // outcome to show; one nobody typed is a notice, never the person's
        // message, and a scheduled task's says it is one (ov-452). Long ones fold.
        if (turn.harness != null) {
            // A report, a task's notice or a reminder (ov-474): its own row.
            HarnessTurn(turn.harness)
        } else if (AgentConversation.isScheduled(turn) && prompt.isNotEmpty()) {
            val quiet = MaterialTheme.colorScheme.onSurfaceVariant
            Column(Modifier.fillMaxWidth().testTag("native-scheduled-turn"), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    Icon(Icons.Outlined.Schedule, null, Modifier.size(14.dp), tint = quiet)
                    Text(AgentConversation.SCHEDULED_TASK, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.Medium, color = quiet)
                }
                FoldedText(prompt, lines = 3, style = MaterialTheme.typography.bodyMedium.copy(color = quiet))
            }
        } else if (AgentConversation.isNotice(turn) && prompt.isNotEmpty()) {
            FoldedText(
                AgentConversation.noticeText(turn),
                lines = 3,
                textAlign = TextAlign.Center,
                style = MaterialTheme.typography.labelMedium.copy(color = MaterialTheme.colorScheme.onSurfaceVariant),
                modifier = Modifier.fillMaxWidth().testTag("native-notice-turn"),
            )
        } else if (prompt.isNotEmpty()) {
            Bubble(prompt, Modifier.testTag("native-prompt"))
        }
        // A notice's own turn says no time of its own: it's the line above, not
        // a message the person sent. Nor does a scheduled task's.
        // Nor does a report's (a hook's turn for one is `Typed` until the transcript says).
        if (!AgentConversation.isNotice(turn) && !AgentConversation.isScheduled(turn) && !AgentConversation.isHarness(turn)) TurnStatus(turn)
    }
}

@Composable
private fun Bubble(text: String, modifier: Modifier = Modifier) {
    Row(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
        Spacer(Modifier.width(48.dp))
        FoldedText(
            text,
            modifier = Modifier
                .weight(1f, fill = false)
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
        AgentConversation.originNote(turn)?.let { Text(it, style = style, color = quiet) }
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
internal fun ThinkingRow(thinking: AgentRow.Thinking) {
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
internal fun StatusMark(status: AgentRow.Status) {
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
private fun SubagentRow(subagent: AgentRow.Subagent) {
    // Unfilled and secondary, as a tool row is: the agent's machinery sits below
    // its words (ov-452).
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            StatusMark(subagent.status)
            Text(AgentConversation.agentType(subagent.agentType), style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
            AgentConversation.toolsLabel(subagent.toolCount)?.let {
                Text(it, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("native-subagent-tools"))
            }
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
internal fun attentionCard(attention: Boolean): Modifier {
    val shape = RoundedCornerShape(Radius.medium)
    return Modifier
        .fillMaxWidth()
        .clip(shape)
        .background(MaterialTheme.colorScheme.surfaceContainerHigh)
        .then(if (attention) Modifier.border(1.dp, MaterialTheme.colorScheme.tertiary.copy(alpha = 0.55f), shape) else Modifier)
        .padding(12.dp)
}

/**
 * A message in claude's own queue (R-29), from its transcript or sent from here a
 * moment ago. [sendNow] (ov-368): claude takes every waiting message now, rather
 * than when its turn ends; offered on a waiting message only.
 */
@Composable
fun QueuedLine(text: String, state: String = "Waiting", sendNow: (() -> Unit)? = null) {
    Column(
        Modifier.fillMaxWidth().testTag("native-queued"),
        horizontalAlignment = Alignment.End,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Bubble(text)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                AgentConversation.queuedLabel(state),
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.Medium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (state == "Waiting" && sendNow != null) {
                TextButton(onClick = sendNow, modifier = Modifier.testTag("native-send-now")) { Text("Send now") }
            }
        }
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
