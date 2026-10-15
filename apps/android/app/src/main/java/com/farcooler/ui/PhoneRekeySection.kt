package com.farcooler.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import com.farcooler.account.DeviceKeys
import com.farcooler.account.PhoneRekey
import com.farcooler.account.RunnerMove
import com.farcooler.data.Identity
import kotlinx.coroutines.launch

/**
 * "Make a new key" under Devices and runners (ov-569). See [PhoneRekey].
 * Sentence case, as Android copy is.
 */
@Composable
fun PhoneRekeySection(model: AppModel, onShowCode: () -> Unit) {
    val scope = rememberCoroutineScope()
    val rekey = model.rekey
    val status by rekey.status.collectAsState()
    val signedIn by model.account.userId.collectAsState()
    var removed by remember { mutableStateOf(false) }
    var asking by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        val own = Identity.publicKey
        removed = DeviceKeys(model.account::directoryPost).list(includingRemoved = true)
            ?.firstOrNull { it.isHeldBy(own) }?.state == "revoked"
    }

    SectionTitle("This phone’s key")
    val started = status.working || status.finished || status.failure != null || rekey.isUnderway
    if (!started) {
        Note(PhoneRekey.invitation(removed), attention = removed)
        TextButton(onClick = { asking = true }) { Text("Make a new key") }
    } else {
        Line("New key", if (rekey.isUnderway || status.finished) "Made on this phone." else "Making a new key…")
        Line("Your account", PhoneRekey.accountLine(signedIn.isNotEmpty(), status))
        val left = rekey.runnersLeft.map { it.id }.toSet()
        for (runner in rekey.allRunners) {
            val move = status.moves[runner.id] ?: if (runner.id in left) RunnerMove.NOT_REACHED else RunnerMove.WAITING
            Line(runner.name, move.detail)
        }
        if (signedIn.isNotEmpty()) {
            Line("Old key", if (status.oldRemoved) "Removed from your account." else "Removed from your account once the runners have moved.")
        }
        status.failure?.let { Note(it.message, attention = true) }
        if (status.working) {
            LinearProgressIndicator()
        } else {
            if (status.needsScan) TextButton(onClick = onShowCode) { Text("Show this phone’s code") }
            TextButton(onClick = { scope.launch { rekey.run() } }) {
                Text(if (status.finished) "Check again" else "Continue")
            }
            if (rekey.runnersLeft.isNotEmpty()) {
                Note(PhoneRekey.FORGET_WARNING)
                TextButton(onClick = { rekey.forgetOldKey() }) { Text("Forget the old key") }
            }
        }
    }

    if (asking) {
        AlertDialog(
            onDismissRequest = { asking = false },
            title = { Text("Make a new key for this phone?") },
            text = { Text(PhoneRekey.CONFIRMATION) },
            confirmButton = {
                TextButton(onClick = {
                    asking = false
                    scope.launch { rekey.run() }
                }) { Text("Make a new key") }
            },
            dismissButton = { TextButton(onClick = { asking = false }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun Line(title: String, detail: String) {
    Column {
        Text(title, style = MaterialTheme.typography.bodyMedium)
        Text(detail, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun Note(text: String, attention: Boolean = false) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = if (attention) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
    )
}
