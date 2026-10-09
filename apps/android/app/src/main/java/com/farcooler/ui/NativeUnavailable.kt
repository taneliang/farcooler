package com.farcooler.ui

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Forum
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import com.farcooler.model.AgentConversation

/**
 * The conversation switch where the view isn't offered, on a Claude or Codex
 * pane, for a reason about its runner (ov-443): the same glyph, dimmed, in the
 * same place, and a tap says why. Any other pane has no switch at all.
 */
@Composable
fun NativeUnavailableButton(
    reason: AgentConversation.Unavailable,
    modifier: Modifier = Modifier,
    maySetSetting: Boolean = true,
) {
    var explaining by remember { mutableStateOf(false) }
    IconButton(
        onClick = { explaining = true },
        modifier = modifier.testTag("native-switch-unavailable").semantics { stateDescription = reason.sentence(maySetSetting) },
    ) {
        Icon(
            Icons.Outlined.Forum,
            contentDescription = "Conversation unavailable",
            tint = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f),
        )
    }
    if (explaining) {
        AlertDialog(
            onDismissRequest = { explaining = false },
            confirmButton = { TextButton(onClick = { explaining = false }) { Text("OK") } },
            title = { Text("Conversation unavailable") },
            text = { Text(reason.sentence(maySetSetting)) },
        )
    }
}
