package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.farcooler.model.LfsNotice
import kotlinx.coroutines.launch

/**
 * "Some large files weren't downloaded." at the top of a worktree's Changes,
 * with Try again (ov-199).
 *
 * What it says and when it shows is [LfsNotice]; [onRetry] asks the runner to
 * try again and returns when it has answered; a refusal is said by the app's
 * action notices, as every host call's is. The count comes back through the
 * fleet, so the card clears itself when the files arrive and stays when they
 * didn't. Neutral, not an error: nothing is wrong with the diff.
 */
@Composable
fun LfsNoticeCard(notice: LfsNotice, onRetry: suspend () -> Unit, modifier: Modifier = Modifier) {
    val scope = rememberCoroutineScope()
    var working by remember { mutableStateOf(false) }
    Column(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Radius.medium))
            .background(MaterialTheme.colorScheme.surfaceContainerLow)
            .padding(12.dp)
            .testTag("changes-lfs-notice"),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(LfsNotice.TITLE, style = MaterialTheme.typography.titleSmall)
        Text(
            LfsNotice.DETAIL,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (notice.canRetry) {
            TextButton(
                onClick = {
                    working = true
                    scope.launch {
                        onRetry()
                        working = false
                    }
                },
                enabled = !working,
                modifier = Modifier.testTag("changes-lfs-retry"),
            ) { Text(if (working) LfsNotice.RETRYING else LfsNotice.RETRY) }
        }
    }
}
