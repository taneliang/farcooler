package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.farcooler.model.DraftHold
import com.farcooler.model.HeldDraft
import kotlinx.coroutines.launch

/**
 * A draft held behind a dialog in this pane (ov-385): "Waiting for the
 * dialog to close" with Withdraw while it waits, then "Sent", or why not.
 * Over the orchestrator's terminal, where Ask the Orchestrator, Reverse and
 * Discuss go after the runner holds their draft. Read from the pane's
 * `draftHold`, so a draft another device sent shows here too.
 */
@Composable
fun HeldDraftBar(hold: DraftHold?, withdraw: suspend (DraftHold) -> Boolean) {
    var tracked by remember { mutableStateOf<String?>(null) }
    var withdrawing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(hold) {
        if (hold != null && hold.isWaiting) tracked = hold.id
        withdrawing = false
    }
    val status = tracked?.let { HeldDraft.status(it, hold) } ?: return
    val title = HeldDraft.title(status) ?: return
    Surface(tonalElevation = 2.dp, modifier = Modifier.fillMaxWidth().testTag("held-draft")) {
        Row(
            Modifier.padding(horizontal = 16.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            when (status) {
                HeldDraft.Status.WAITING -> CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                HeldDraft.Status.SENT -> Icon(Icons.Outlined.CheckCircleOutline, null)
                else -> Icon(Icons.Outlined.ErrorOutline, null)
            }
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleSmall)
                HeldDraft.detail(status)?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            if (status == HeldDraft.Status.WAITING && hold != null) {
                TextButton(
                    onClick = {
                        withdrawing = true
                        // Not reached: Withdraw is there to press again.
                        scope.launch { if (!withdraw(hold)) withdrawing = false }
                    },
                    enabled = !withdrawing,
                    modifier = Modifier.testTag("held-draft-withdraw"),
                ) { Text(HeldDraft.WITHDRAW) }
            } else {
                IconButton(onClick = { tracked = null }) { Icon(Icons.Outlined.Close, contentDescription = "Dismiss") }
            }
        }
    }
}
