package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.ceremony.CeremonyRunner
import com.farcooler.ceremony.TeammateJoinCopy

/**
 * A reply from another account's runner admin, held until the person holding
 * this device says Join (ov-543). Nothing is added before that: these are
 * someone else's runners, on the Claude account signed in there.
 *
 * The phone and the Mac ask in the same words ([TeammateJoinCopy]).
 */
@Composable
fun TeammateJoinPrompt(
    from: String,
    runners: List<CeremonyRunner>,
    onJoin: () -> Unit,
    onCancel: () -> Unit,
) {
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            TeammateJoinCopy.title(runners),
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.testTag("join-teammate-title"),
        )
        Text(
            TeammateJoinCopy.detail(from),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.testTag("join-teammate-detail"),
        )
        if (runners.size > 1) {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                for (runner in runners) {
                    // As its owner named it: one line, cut in the middle.
                    Text(
                        runner.label,
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 1,
                        overflow = TextOverflow.MiddleEllipsis,
                    )
                }
            }
        }
        Button(
            onClick = onJoin,
            modifier = Modifier.fillMaxWidth().testTag("join-teammate-join"),
        ) { Text("Join") }
        OutlinedButton(
            onClick = onCancel,
            modifier = Modifier.fillMaxWidth().testTag("join-teammate-cancel"),
        ) { Text("Cancel") }
    }
}
