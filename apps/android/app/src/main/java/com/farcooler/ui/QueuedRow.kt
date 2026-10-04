package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.unit.dp
import com.farcooler.model.QueueControls
import com.farcooler.model.QueuedPrompt

/** A message written but not yet sent. */
@Composable
internal fun QueuedRow(
    queued: QueuedPrompt,
    onEdit: (String) -> Unit,
    onCancel: () -> Unit,
    onSteer: () -> Unit,
    controls: QueueControls,
) {
    val enabled = controls.isAvailable
    var editing by remember { mutableStateOf(false) }
    var draft by remember(queued.id) { mutableStateOf(queued.text) }

    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(14.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(horizontal = 12.dp, vertical = 8.dp),
    ) {
        if (editing) {
            BasicTextField(
                value = draft,
                onValueChange = { draft = it },
                textStyle = LocalTextStyle.current.copy(
                    color = MaterialTheme.colorScheme.onSurface,
                ),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                modifier = Modifier.fillMaxWidth(),
            )
        } else if (queued.text.isEmpty() && queued.imageCount > 0) {
            // An image with no words is still a message. Without this the
            // bubble was empty and read as a dropped attachment.
            Text(
                if (queued.imageCount == 1) "1 image" else "${queued.imageCount} images",
                style = MaterialTheme.typography.bodyMedium,
            )
        } else {
            Text(queued.text, style = MaterialTheme.typography.bodyMedium)
        }

        Row(
            Modifier.padding(top = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                "Queued",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            // The queue's whole point is that a message you can still see and
            // still edit beats one already gone — so waiting is the default.
            // But a message written mid-turn is very often a correction, and a
            // correction is worth nothing once the wrong thing has been done.
            Action("Send now", enabled, onSteer)
            Action(if (editing) "Save" else "Edit", enabled = enabled) {
                if (editing) {
                    editing = false
                    val trimmed = draft.trim()
                    if (trimmed.isNotEmpty() && trimmed != queued.text) onEdit(trimmed)
                } else {
                    draft = queued.text
                    editing = true
                }
            }
            Action("Remove", enabled, onCancel)
        }
        // Dimmed and said, not hidden (ov-171): see [QueueControls].
        (controls as? QueueControls.Unavailable)?.let {
            Text(
                it.sentence,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 4.dp),
            )
        }
    }
}

@Composable
private fun Action(label: String, enabled: Boolean = true, onClick: () -> Unit) {
    Text(
        label,
        style = MaterialTheme.typography.labelSmall,
        color = if (enabled) {
            MaterialTheme.colorScheme.primary
        } else {
            MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f)
        },
        modifier = Modifier.clickable(enabled = enabled, onClick = onClick),
    )
}
