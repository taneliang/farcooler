package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.farcooler.model.TaskSpend
import com.farcooler.model.TaskUsageFormat
import com.farcooler.model.TaskUsageState

/**
 * A task's Usage section (ov-195): what its agents spent, read when the task
 * opens. Quiet on purpose, secondary text and no color beyond the section's own
 * header, since nothing here needs you. The words are [TaskUsageFormat]'s, the
 * same the Mac, iOS and `farcooler report` say.
 */
fun LazyListScope.taskUsageItems(state: TaskUsageState, onRetry: () -> Unit, header: @Composable () -> Unit) {
    when (state) {
        TaskUsageState.NeedsUpdate -> item(key = "usage") {
            header()
            Quiet(TaskUsageFormat.NEEDS_UPDATE, Modifier.padding(horizontal = 16.dp).testTag("task-usage-needs-update"))
        }
        TaskUsageState.Failed -> item(key = "usage") {
            header()
            Quiet(TaskUsageFormat.COULDNT_READ, Modifier.padding(horizontal = 16.dp))
            TextButton(onClick = onRetry, modifier = Modifier.padding(horizontal = 4.dp).testTag("task-usage-retry")) {
                Text(TaskUsageFormat.TRY_AGAIN)
            }
        }
        TaskUsageState.Loading -> item(key = "usage") {
            header()
            CircularProgressIndicator(
                Modifier.padding(horizontal = 16.dp, vertical = 8.dp).size(18.dp).testTag("task-usage-loading"),
                strokeWidth = 2.dp,
            )
        }
        is TaskUsageState.Loaded -> {
            val usage = state.usage
            item(key = "usage") {
                header()
                if (usage.totals.isEmpty) {
                    Quiet(TaskUsageFormat.NOTHING_YET, Modifier.padding(horizontal = 16.dp).testTag("task-usage-empty"))
                } else {
                    Totals(usage.totals)
                }
            }
            if (!usage.totals.isEmpty && usage.byHarnessModel.isNotEmpty()) {
                item(key = "usage-breakdown") { Breakdown(usage.rows) }
            }
        }
    }
}

@Composable
private fun Totals(t: TaskSpend) {
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("task-usage"),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(TaskUsageFormat.tokensLine(t), style = MaterialTheme.typography.bodyLarge)
        TaskUsageFormat.tokenDetail(t)?.let { Quiet(it) }
        Quiet(TaskUsageFormat.cost(t))
        TaskUsageFormat.time(t)?.let { Quiet(it) }
    }
}

@Composable
private fun Breakdown(rows: List<com.farcooler.model.TaskSpendRow>) {
    var open by rememberSaveable { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth().testTag("task-usage-breakdown")) {
        ListItem(
            headlineContent = { Text("By Harness and Model", style = MaterialTheme.typography.bodyMedium) },
            leadingContent = {
                Icon(
                    if (open) Icons.Outlined.KeyboardArrowDown else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            },
            modifier = Modifier
                .heightIn(min = 48.dp)
                .clickable(role = Role.Button, onClickLabel = if (open) "Hide" else "Show") { open = !open },
        )
        if (open) {
            for (row in rows) {
                ListItem(
                    headlineContent = { Text(row.title, style = MaterialTheme.typography.bodyMedium) },
                    supportingContent = { Text(TaskUsageFormat.detail(row)) },
                )
            }
        }
    }
}

@Composable
private fun Quiet(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier,
    )
}
