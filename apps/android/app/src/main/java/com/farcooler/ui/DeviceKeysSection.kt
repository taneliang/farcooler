package com.farcooler.ui

import kotlinx.coroutines.withContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.Dispatchers
import androidx.compose.runtime.produceState
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
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
import com.farcooler.account.DeviceKeyRow
import com.farcooler.account.DeviceKeys
import com.farcooler.data.Identity
import kotlinx.coroutines.launch

/**
 * The account's device keys, each with Remove but this phone's own (ov-549
 * review M1). See [DeviceKeys].
 */
@Composable
fun DeviceKeysSection(model: AppModel) {
    val scope = rememberCoroutineScope()
    val keys = remember(model) { DeviceKeys(model.account::directoryPost) }
    var rows by remember { mutableStateOf<List<DeviceKeyRow>?>(emptyList()) }
    var removing by remember { mutableStateOf<DeviceKeyRow?>(null) }
    var removalFailed by remember { mutableStateOf(false) }
    val own by produceState<String?>(null) {
        while (true) {
            value = withContext(Dispatchers.IO) { Identity.publicKey }
            if (value != null) break
            delay(3_000)
        }
    }

    LaunchedEffect(Unit) { rows = keys.list() }

    SectionTitle("Device keys")
    val current = rows
    when {
        current == null -> Hint("Couldn’t load your device keys.")
        current.isEmpty() -> Hint("No device keys yet. Each device adds its key when it signs in.")
        else -> for (row in current) {
            val mine = row.isHeldBy(own)
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(row.label, style = MaterialTheme.typography.bodyMedium)
                    Text(
                        row.identity(),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        row.detail(isThisDevice = mine, anyConfirmed = current.any { it.isConfirmed }),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (!mine) TextButton(onClick = { removing = row }) { Text("Remove") }
            }
        }
    }
    if (removalFailed) Hint("Couldn’t remove that device. Check your connection, then try again.")
    Hint(
        "Each device that signs in to Far Cooler adds its key here, so your team’s admins can add it " +
            "to runners as yours. To tell two devices apart, compare the words under each with the ones " +
            "that device shows for itself here. Remove any device you don’t recognize.",
    )

    removing?.let { row ->
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text("Remove “${row.shownLabel}”?") },
            text = { Text(DeviceKeyRow.removalWarning(row, current.orEmpty())) },
            confirmButton = {
                TextButton(onClick = {
                    removing = null
                    scope.launch {
                        removalFailed = !keys.remove(row.id)
                        rows = keys.list()
                        // That was the last confirmed device: this phone,
                        // still pending, is the one being kept (R-75).
                        if (!removalFailed && DeviceKeyRow.startsOverHere(rows.orEmpty(), own)) {
                            model.registerDeviceAgain()
                            rows = keys.list()
                        }
                    }
                }) { Text("Remove device") }
            },
            dismissButton = { TextButton(onClick = { removing = null }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun Hint(text: String) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}
