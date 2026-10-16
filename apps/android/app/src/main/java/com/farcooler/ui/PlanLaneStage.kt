package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.GlancePalette
import com.farcooler.model.PlanLane
import com.farcooler.model.PlanPrStage
import com.farcooler.model.PrActions
import kotlinx.coroutines.launch

/**
 * A lane's pull request on its page (ov-317): the stage in words (amber, and leading with the problem, when a person
 * has to act), the pull request and its open threads, Open on GitHub (a github.com link, which opens in GitHub when
 * it is installed), and Mark ready for a draft. [markReady] is null where this device may not act on pull requests: a
 * runner without `pr_actions`, or a Read grant. The iPhone's `PlanLaneStageBlock`.
 */
@Composable
fun PlanLaneStage(
    lane: PlanLane,
    stage: PlanPrStage,
    markReady: (suspend (PlanLane, PlanPrStage) -> String?)?,
    modifier: Modifier = Modifier,
) {
    val amber = glanceColor(GlancePalette.amber)
    val links = LocalUriHandler.current
    val scope = rememberCoroutineScope()
    var asking by remember { mutableStateOf(false) }
    var sending by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<String?>(null) }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        stage.words?.let {
            Text(
                it,
                style = MaterialTheme.typography.bodyMedium,
                color = if (stage.needsAttention) amber else MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
                modifier = Modifier.testTag(if (stage.needsAttention) "plan-lane-stage-attention" else "plan-lane-stage"),
            )
        }
        stage.detail?.let {
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("plan-lane-stage-detail"))
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
            stage.link?.let { link ->
                OutlinedButton(onClick = { links.openUri(link) }, modifier = Modifier.testTag("plan-lane-review-on-github")) {
                    Text(PrActions.OPEN_ON_GITHUB)
                }
            }
            if (stage.canMarkReady && markReady != null) {
                OutlinedButton(onClick = { asking = true }, enabled = !sending, modifier = Modifier.testTag("plan-lane-mark-ready")) {
                    Text(PrActions.MARK_READY)
                }
                if (sending) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
            }
        }
        failure?.let {
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("plan-lane-mark-ready-failure"))
        }
    }
    if (asking && markReady != null) {
        AlertDialog(
            onDismissRequest = { asking = false },
            title = { Text(PrActions.readyTitle(stage)) },
            text = { Text(PrActions.READY_MESSAGE) },
            confirmButton = {
                TextButton(
                    onClick = {
                        asking = false
                        sending = true
                        failure = null
                        scope.launch {
                            failure = markReady(lane, stage)
                            sending = false
                        }
                    },
                    modifier = Modifier.testTag("plan-lane-mark-ready-confirm"),
                ) { Text(PrActions.MARK_READY) }
            },
            dismissButton = { TextButton(onClick = { asking = false }) { Text("Cancel") } },
            modifier = Modifier.testTag("plan-lane-mark-ready-dialog"),
        )
    }
}
