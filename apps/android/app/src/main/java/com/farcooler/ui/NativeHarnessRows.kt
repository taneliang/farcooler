package com.farcooler.ui

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.clickable
import androidx.compose.ui.draw.drawBehind
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ExpandLess
import androidx.compose.material.icons.outlined.ExpandMore
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Constraints
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.AgentConversation
import com.farcooler.model.AgentRow

// What the conversation view draws for a turn claude wrote for itself (ov-474): a
// subagent's report, another session's message, a background task's notice and the
// harness's reminders. None is the person's message, so none is a bubble, and none
// shows XML. The projector read them (`harness.rs`); this draws what it says, as the
// Mac's and the iPhone's do. The Android view has no agent transcripts to open
// (ov-453 is the Mac's and the iPhone's), so a report has no link here.

@Composable
internal fun HarnessTurn(harness: AgentRow.Harness) {
    when (harness.kind) {
        "Status" -> Text(
            AgentConversation.harnessTitle(harness),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.fillMaxWidth().testTag("native-harness-status"),
        )
        "Note" -> HarnessNote(harness)
        else -> HarnessReport(harness)
    }
}

/** "Report from <what it was asked>", its words under that folded after a few lines. */
@Composable
private fun HarnessReport(harness: AgentRow.Harness) {
    val quiet = MaterialTheme.colorScheme.onSurfaceVariant
    val rule = MaterialTheme.colorScheme.outlineVariant
    Column(
        Modifier
            .fillMaxWidth()
            .testTag("native-harness-report")
            .drawBehind { drawLine(rule, Offset(1.dp.toPx(), 0f), Offset(1.dp.toPx(), size.height), strokeWidth = 2.dp.toPx()) }
            .padding(start = 12.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            AgentConversation.harnessTitle(harness),
            style = MaterialTheme.typography.labelMedium,
            fontWeight = FontWeight.Medium,
            color = quiet,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (harness.body.isNotEmpty()) FoldedMarkdown(harness.body, lines = AgentConversation.HARNESS_LINES)
    }
}

/** The harness's reminders: one muted row, closed until asked for. */
@Composable
private fun HarnessNote(harness: AgentRow.Harness) {
    var open by remember(harness.body) { mutableStateOf(false) }
    val quiet = MaterialTheme.colorScheme.onSurfaceVariant
    Column(Modifier.fillMaxWidth().testTag("native-harness-note"), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(
            Modifier
                .clickable(role = Role.Button) { open = !open }
                .semantics { stateDescription = if (open) "Expanded" else "Collapsed" }
                .testTag("native-harness-note-toggle"),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Icon(if (open) Icons.Outlined.ExpandLess else Icons.Outlined.ExpandMore, null, Modifier.size(16.dp), tint = quiet)
            Text(AgentConversation.harnessTitle(harness), style = MaterialTheme.typography.labelMedium, color = quiet)
        }
        if (open) {
            Text(
                harness.body,
                style = MaterialTheme.typography.labelMedium,
                color = quiet,
                modifier = Modifier.padding(start = 20.dp).testTag("native-harness-note-body"),
            )
        }
    }
}

/**
 * A report's words, drawn as the agent's own prose is ([MarkdownText]: lists,
 * headings, code, bold), folded to [lines] lines with Show more for the rest.
 * Blocks take no line limit as one Text does, so the fold is a height: [lines]
 * lines of body text, clipping the whole, with the last line fading so none is
 * cut through. Whether it overflows is what the layout measured at this width.
 */
@Composable
internal fun FoldedMarkdown(text: String, modifier: Modifier = Modifier, lines: Int = AgentConversation.COLLAPSED_LINES) {
    var open by remember(text) { mutableStateOf(false) }
    var overflows by remember(text) { mutableStateOf(false) }
    val lineHeight = MaterialTheme.typography.bodyMedium.lineHeight
    val capPx = with(LocalDensity.current) { lineHeight.toPx() * lines }
    val fadePx = with(LocalDensity.current) { lineHeight.toPx() }
    Column(modifier.animateContentSize()) {
        MarkdownText(
            text,
            modifier = Modifier
                .fillMaxWidth()
                .testTag("native-folded-markdown")
                .layout { measurable, constraints ->
                    val placeable = measurable.measure(constraints.copy(minHeight = 0, maxHeight = Constraints.Infinity))
                    val folded = !open && placeable.height > capPx
                    overflows = placeable.height > capPx
                    val height = if (folded) capPx.toInt() else placeable.height
                    layout(placeable.width, height) { placeable.placeRelative(0, 0) }
                }
                .clipToBounds()
                .graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
                .drawWithContent {
                    drawContent()
                    if (!open && overflows) {
                        drawRect(
                            Brush.verticalGradient(listOf(Color.Transparent, Color.Black), startY = size.height - fadePx, endY = size.height),
                            blendMode = BlendMode.DstOut,
                        )
                    }
                },
        )
        if (open || overflows) {
            TextButton(onClick = { open = !open }, modifier = Modifier.testTag("native-show-more")) {
                Text(if (open) "Show less" else "Show more")
            }
        }
    }
}
