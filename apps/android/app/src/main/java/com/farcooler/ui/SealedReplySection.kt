package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.ceremony.SealedReply

/**
 * The code, under the reply's QR code, while a reply sealed for a browser is
 * out (ov-770): the six characters to match against the browser's screen, and
 * each runner's host key one tap behind them. Nothing at all for a reply that
 * wasn't sealed, so the QR-only screen is the one it always was.
 */
@Composable
fun SealedReplySection(reply: SealedReply, deviceName: String) {
    val state by reply.state.collectAsStateWithLifecycle()
    val code by reply.code.collectAsStateWithLifecycle()
    val hostKeys by reply.hostKeys.collectAsStateWithLifecycle()
    var showKeys by remember { mutableStateOf(false) }
    val name = deviceName.ifBlank { "the new device" }

    val shown = code
    when {
        state == SealedReply.State.NONE -> Unit
        state == SealedReply.State.NOT_SENT || shown == null -> Text(
            "Couldn’t send this to $name. If it has a camera, scan the code above with it.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.tertiary,
            textAlign = TextAlign.Center,
        )
        else -> Column(
            Modifier.fillMaxWidth(),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(
                if (state == SealedReply.State.SENT) "Sent to $name. Check that it shows this code:"
                else "Sending to $name…",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Text(
                if (shown.length == 6) "${shown.take(3)} ${shown.takeLast(3)}" else shown,
                style = MaterialTheme.typography.headlineMedium,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.SemiBold,
            )
            if (hostKeys.isNotEmpty()) {
                Text(
                    if (showKeys) "Hide host keys" else "Show host keys",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.clickable { showKeys = !showKeys },
                )
                if (showKeys) {
                    hostKeys.forEach { (label, hostKey) ->
                        Text(
                            "$label: ${hostKey.ifEmpty { "not pinned" }}",
                            style = MaterialTheme.typography.bodySmall,
                            fontFamily = FontFamily.Monospace,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}
