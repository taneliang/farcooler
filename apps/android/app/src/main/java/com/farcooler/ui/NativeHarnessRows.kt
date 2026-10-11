package com.farcooler.ui

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
        if (harness.body.isNotEmpty()) FoldedText(harness.body, lines = AgentConversation.HARNESS_LINES)
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
