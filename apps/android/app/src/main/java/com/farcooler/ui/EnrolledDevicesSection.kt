package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.EnrolledDevices

/**
 * The devices that may log in to this runner, each with who added it, and a Revoke that offers to take what the
 * device added too (ov-855). The Mac's and the iPhone's "Devices on This Runner", in this platform's controls.
 * [revoke] takes the device and whether the ones it added go with it; [failure] is why the last one didn't take.
 */
@Composable
fun EnrolledDevicesSection(
    devices: EnrolledDevices,
    failure: String?,
    revoke: (EnrolledDevices.Device, Boolean) -> Unit,
) {
    var asking by remember { mutableStateOf<EnrolledDevices.RevokeChoices?>(null) }
    SectionTitle(EnrolledDevices.SECTION_TITLE)
    if (devices.devices.isEmpty()) {
        Text(EnrolledDevices.EMPTY, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
    for (device in devices.devices) {
        Row(
            Modifier.fillMaxWidth().testTag("enrolled-device-${device.clientId}"),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(Modifier.weight(1f)) {
                Text(device.title, maxLines = 2, overflow = TextOverflow.Ellipsis)
                devices.addedByLine(device)?.let {
                    Text(
                        it,
                        modifier = Modifier.testTag("enrolled-device-added-by-${device.clientId}"),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            TextButton(
                onClick = { asking = devices.revokeChoices(device) },
                modifier = Modifier.testTag("enrolled-device-revoke-${device.clientId}"),
            ) { Text("Revoke…") }
        }
    }
    failure?.let {
        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
    asking?.let { choices ->
        val destructive = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error)
        AlertDialog(
            onDismissRequest = { asking = null },
            title = { Text(choices.title) },
            text = { Text(choices.message) },
            confirmButton = {
                Column(horizontalAlignment = Alignment.End) {
                    choices.withAddedTitle?.let { title ->
                        TextButton(
                            colors = destructive,
                            modifier = Modifier.testTag("enrolled-device-revoke-with-added"),
                            onClick = { asking = null; revoke(choices.device, true) },
                        ) { Text(title) }
                    }
                    TextButton(
                        colors = destructive,
                        modifier = Modifier.testTag("enrolled-device-revoke-this"),
                        onClick = { asking = null; revoke(choices.device, false) },
                    ) { Text(EnrolledDevices.JUST_THIS_TITLE) }
                }
            },
            dismissButton = { TextButton(onClick = { asking = null }) { Text("Cancel") } },
        )
    }
}
