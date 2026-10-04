package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.farcooler.net.InputHold

/**
 * The one quiet line under a terminal when typed input didn't get through
 * (ov-238). Held input offers Try again and Discard; typing that may have
 * arrived offers neither, because sending it again could type it twice. Not an
 * alert: the pane is fine and the person is mid-keystroke.
 */
@Composable
fun InputHoldLine(
    line: InputHold.Line,
    onRetry: () -> Unit,
    onDiscard: () -> Unit,
    onDismiss: () -> Unit,
) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceContainer,
        modifier = Modifier.fillMaxWidth().testTag("terminal-unsent"),
    ) {
        Row(
            Modifier.padding(start = 16.dp, end = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                line.sentence,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f).padding(vertical = 8.dp),
            )
            if (line.holding) {
                TextButton(onClick = onRetry, modifier = Modifier.testTag("terminal-unsent-retry")) {
                    Text(InputHold.RETRY)
                }
                TextButton(onClick = onDiscard, modifier = Modifier.testTag("terminal-unsent-discard")) {
                    Text(InputHold.DISCARD)
                }
            } else {
                TextButton(onClick = onDismiss, modifier = Modifier.testTag("terminal-unsent-dismiss")) {
                    Text(InputHold.DISMISS)
                }
            }
        }
    }
}
