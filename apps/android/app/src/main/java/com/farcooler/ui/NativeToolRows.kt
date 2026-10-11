package com.farcooler.ui

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.outlined.ExpandLess
import androidx.compose.material.icons.outlined.ExpandMore
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material.icons.outlined.Timelapse
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.AgentConversation
import com.farcooler.model.AgentRow

// The agent's machinery in the conversation view (ov-452, ov-459), set below its
// words as the Mac's and the iPhone's are: tool calls, runs of them folded into
// one line, and the task list. Secondary and one line until tapped; each call
// opens to what it was asked and what came back.

/**
 * A row's header that opens what's under it: the whole line is the target, at
 * least 48 dp tall, with a chevron at its end that says which way it goes.
 */
@Composable
private fun OpeningHeader(
    open: Boolean,
    opens: Boolean,
    description: String,
    toggle: () -> Unit,
    time: @Composable () -> Unit,
    content: @Composable (Modifier) -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 48.dp)
            .then(
                if (opens) {
                    Modifier.clickable(onClickLabel = if (open) "Hide details" else "Show details", role = Role.Button, onClick = toggle)
                } else {
                    Modifier
                },
            )
            .semantics(mergeDescendants = true) {
                contentDescription = description
                if (opens) stateDescription = if (open) "Expanded" else "Collapsed"
            },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        content(Modifier.weight(1f))
        if (opens) {
            Icon(
                if (open) Icons.Outlined.ExpandLess else Icons.Outlined.ExpandMore,
                contentDescription = null,
                modifier = Modifier.size(18.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        time()
    }
}

/** One tool call, opening to its input, its result and an edit's diff. */
@Composable
internal fun NativeToolRow(id: String, tool: AgentRow.Tool, startOpen: Boolean = false) {
    var open by remember { mutableStateOf(startOpen) }
    Column(Modifier.fillMaxWidth().animateContentSize().testTag("native-tool-$id")) {
        OpeningHeader(
            open = open,
            opens = tool.opens,
            description = listOf(tool.name, tool.summary).filter { it.isNotEmpty() }.joinToString(", "),
            toggle = { open = !open },
            time = { RunTimeChip(tool.startedMs, tool.endedMs) },
        ) { weighted ->
            Row(weighted, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                StatusMark(tool.status)
                Text(tool.name, style = MaterialTheme.typography.bodySmall, fontWeight = FontWeight.Medium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                // Prose wraps to two lines and cuts at its end; only a path is cut in the middle.
                Text(
                    tool.summary,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.8f),
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
            }
        }
        if (open) NativeToolDetail(tool, Modifier.padding(start = 24.dp, bottom = 8.dp))
    }
}

/** What a call opens to. */
@Composable
private fun NativeToolDetail(tool: AgentRow.Tool, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth().testTag("native-tool-detail"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        tool.input?.let { DetailPart("Input", it) }
        if (tool.diff.isNotEmpty()) NativeDiff(tool.diff)
        tool.result?.let { DetailPart(if (tool.status == AgentRow.Status.Failed) "Error" else "Result", it) }
    }
}

@Composable
private fun DetailPart(title: String, text: String) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(title, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        SelectionContainer {
            Text(
                text,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(Radius.small))
                    .background(MaterialTheme.colorScheme.surfaceContainerHigh)
                    .padding(8.dp),
            )
        }
    }
}

/** An edit's hunks, as unified-diff lines. */
@Composable
internal fun NativeDiff(hunks: List<AgentRow.Hunk>) {
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

/** A run of tool calls as one line: how many, and what for. Opens to each. */
@Composable
internal fun NativeToolGroupRow(id: String, rows: List<AgentRow>, startOpen: Boolean = false) {
    var open by remember { mutableStateOf(startOpen) }
    val tools = rows.mapNotNull { (it.kind as? AgentRow.Kind.OfTool)?.tool }
    val title = AgentConversation.groupTitle(tools)
    val purpose = AgentConversation.groupPurpose(tools)
    val status = AgentConversation.groupStatus(tools)
    Column(Modifier.fillMaxWidth().animateContentSize().testTag("native-tool-group")) {
        OpeningHeader(
            open = open,
            opens = true,
            description = "$title: $purpose",
            toggle = { open = !open },
            time = {
                RunTimeChip(tools.firstOrNull()?.startedMs, if (status == AgentRow.Status.Running) null else tools.mapNotNull { it.endedMs }.maxOrNull())
            },
        ) { weighted ->
            Row(weighted, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                StatusMark(status)
                Text(
                    title,
                    style = MaterialTheme.typography.bodySmall,
                    fontWeight = FontWeight.Medium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    softWrap = false,
                )
                Text(
                    purpose,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.8f),
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
            }
        }
        if (open) {
            Column(Modifier.padding(start = 24.dp).testTag("native-tool-group-rows")) {
                for (row in rows) {
                    when (val kind = row.kind) {
                        is AgentRow.Kind.OfTool -> NativeToolRow(row.id, kind.tool, startOpen)
                        is AgentRow.Kind.OfThinking -> ThinkingRow(kind.thinking)
                        else -> Unit
                    }
                }
            }
        }
    }
}

/** The agent's task list as a checklist. */
@Composable
internal fun NativeTasksRow(tasks: AgentRow.Tasks) {
    val quiet = MaterialTheme.colorScheme.onSurfaceVariant
    Column(Modifier.fillMaxWidth().testTag("native-tasks"), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Tasks", style = MaterialTheme.typography.bodySmall, fontWeight = FontWeight.Medium, color = quiet)
            Text(AgentConversation.taskProgress(tasks), style = MaterialTheme.typography.bodySmall, color = quiet)
        }
        for (item in tasks.items) {
            val done = item.status == "Completed"
            val doing = item.status == "InProgress"
            Row(
                Modifier.semantics(mergeDescendants = true) { stateDescription = AgentConversation.taskState(item) },
                verticalAlignment = Alignment.Top,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Icon(
                    when {
                        done -> Icons.Filled.CheckCircle
                        doing -> Icons.Outlined.Timelapse
                        else -> Icons.Outlined.RadioButtonUnchecked
                    },
                    contentDescription = null,
                    modifier = Modifier.padding(top = 2.dp).size(16.dp),
                    tint = if (doing) MaterialTheme.colorScheme.primary else quiet.copy(alpha = if (done) 1f else 0.7f),
                )
                Text(
                    item.subject,
                    style = MaterialTheme.typography.bodyMedium,
                    color = if (doing) MaterialTheme.colorScheme.onSurface else quiet,
                    textDecoration = if (done) TextDecoration.LineThrough else null,
                )
            }
        }
    }
}

/**
 * A message that may be long: its first [lines], and Show more for the rest.
 * Whether it runs past them is measured at the width it's drawn at (the layout
 * says it overflowed), so a long line on a narrow screen folds as surely as many
 * short ones (ov-452 review).
 */
@Composable
internal fun FoldedText(
    text: String,
    modifier: Modifier = Modifier,
    lines: Int = AgentConversation.COLLAPSED_LINES,
    style: TextStyle = MaterialTheme.typography.bodyMedium,
    textAlign: TextAlign? = null,
) {
    var open by remember(text) { mutableStateOf(false) }
    var overflows by remember(text) { mutableStateOf(false) }
    Column(modifier.animateContentSize()) {
        SelectionContainer {
            Text(
                text,
                style = style,
                textAlign = textAlign,
                maxLines = if (open) Int.MAX_VALUE else lines,
                overflow = TextOverflow.Ellipsis,
                onTextLayout = { if (!open) overflows = it.hasVisualOverflow },
                modifier = if (textAlign != null) Modifier.fillMaxWidth() else Modifier,
            )
        }
        if (open || overflows) {
            TextButton(onClick = { open = !open }, modifier = Modifier.testTag("native-show-more")) {
                Text(if (open) "Show less" else "Show more")
            }
        }
    }
}
