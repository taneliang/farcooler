package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
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
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.PeopleAccess
import com.farcooler.model.PersonAccess
import com.farcooler.model.StepUpKeyStanding

/** What the People screen can do on this phone: a null draws no control, so it never offers what the runner would refuse. */
class PeopleAccessActions(
    val removeDevice: ((PeopleAccess.Device, Boolean) -> Unit)? = null,
    val approve: ((PeopleAccess.Device) -> Unit)? = null,
    val allowAdminActions: (() -> Unit)? = null,
)

/**
 * The People screen's Access (ov-856, §8): each person with their Access and their devices under them, then Check
 * These Devices and Waiting for Approval when they have any, then this device's own key. The Mac's and the iPhone's
 * sections, in this platform's sentence case.
 */
@Composable
fun PeopleAccessSection(
    access: PeopleAccess,
    runner: String,
    actions: PeopleAccessActions,
    shownCode: String?,
    note: String?,
) {
    var removing by remember { mutableStateOf<PeopleAccess.Device?>(null) }
    SectionTitle("Access")
    for (person in access.people) {
        Column(Modifier.fillMaxWidth().testTag("people-person-${person.handle.ifEmpty { "owner" }}"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Column(Modifier.weight(1f)) {
                    Text(person.name, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    Text(person.access.summary, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (person.isYou && person.access != PersonAccess.OWNER) {
                    Text("You", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Text(
                    person.access.title,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.testTag("people-access-${person.handle.ifEmpty { "owner" }}"),
                )
            }
            for (device in person.devices) {
                DeviceRow(device, runner, actions, Modifier.padding(start = 12.dp)) { removing = it }
            }
        }
    }
    Text(
        "Admins can add and remove people and their devices. A viewer can see the board, never a terminal.",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    if (access.toCheck.isNotEmpty()) {
        SectionTitle(PeopleAccess.CHECK_HEADER)
        Column(Modifier.testTag("people-check"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            for (device in access.toCheck) DeviceRow(device, runner, actions, Modifier) { removing = it }
        }
        Text(PeopleAccess.CHECK_FOOTER, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
    if (access.shownWaiting.isNotEmpty()) {
        SectionTitle(PeopleAccess.WAITING_HEADER)
        Column(Modifier.testTag("people-waiting"), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            for (device in access.shownWaiting) WaitingRow(device, access, actions)
        }
        Text(PeopleAccess.WAITING_FOOTER, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
    val me = access.thisDevice
    if (me != null && (access.yourAccess == PersonAccess.OWNER || access.yourAccess == PersonAccess.ADMIN)) {
        ThisDeviceAdmin(me, shownCode, actions)
    }
    if (note != null) {
        Text(note, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("people-access-note"))
    }
    removing?.let { device ->
        RemoveDeviceDialog(device, runner, onDismiss = { removing = null }) { withAdded -> actions.removeDevice?.invoke(device, withAdded) }
    }
}

/** What removing [device] takes, asked before the runner is. [remove] is told whether what it added goes too. */
@Composable
fun RemoveDeviceDialog(device: PeopleAccess.Device, runner: String, onDismiss: () -> Unit, remove: (Boolean) -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Remove ${device.name}?") },
        text = { Text(removalMessage(device, runner)) },
        confirmButton = {
            Column {
                TextButton(onClick = { onDismiss(); remove(false) }) { Text("Remove device") }
                if (device.added > 0) {
                    TextButton(onClick = { onDismiss(); remove(true) }) { Text("Remove device and ${device.added} it added") }
                }
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** What removing a device does, said before it is done. */
fun removalMessage(device: PeopleAccess.Device, runner: String): String {
    var line = "${device.name} loses its access to $runner right away."
    if (device.hasShell) line += " Its shell goes too: it can no longer use ssh, git or Zed here." // casing ok: Zed is a product name
    if (device.added > 0) line += if (device.added == 1) " It added one other device." else " It added ${device.added} other devices."
    return line
}

@Composable
private fun DeviceRow(
    device: PeopleAccess.Device,
    runner: String,
    actions: PeopleAccessActions,
    modifier: Modifier,
    remove: (PeopleAccess.Device) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    Row(modifier.fillMaxWidth().testTag("device-${device.id}"), verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(device.name, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            device.addedBy?.let {
                Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2)
            }
            if (device.viewerOnThisDevice) {
                // style-exempt: a capsule badge, fully round at any height
                Surface(shape = RoundedCornerShape(50), color = MaterialTheme.colorScheme.surfaceVariant, modifier = Modifier.testTag("device-viewer-${device.id}")) {
                    Text(
                        PeopleAccess.VIEWER_BADGE,
                        style = MaterialTheme.typography.labelSmall,
                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                    )
                }
            }
            if (device.hasShell) {
                Text(
                    "⚠ ${PeopleAccess.SHELL_BADGE}",
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.Medium,
                    color = MaterialTheme.colorScheme.tertiary,
                )
                Text(
                    PeopleAccess.shellWarning(runner),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.testTag("device-shell-${device.id}"),
                )
            }
        }
        if (actions.removeDevice != null) {
            Box {
                IconButton(
                    onClick = { open = true },
                    modifier = Modifier.testTag("device-menu-${device.id}").semantics { contentDescription = "${device.name} options" },
                ) { Icon(Icons.Filled.MoreVert, contentDescription = null) }
                DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                    DropdownMenuItem(
                        text = { Text(PeopleAccess.REMOVE_DEVICE) },
                        onClick = { open = false; remove(device) },
                        modifier = Modifier.testTag("device-remove-${device.id}"),
                    )
                }
            }
        }
    }
}

@Composable
private fun WaitingRow(device: PeopleAccess.Device, access: PeopleAccess, actions: PeopleAccessActions) {
    Column(Modifier.fillMaxWidth().testTag("waiting-${device.id}"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(device.name, maxLines = 2, overflow = TextOverflow.Ellipsis)
                if (device.personName.isNotEmpty()) {
                    Text(
                        device.personName,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.testTag("waiting-person-${device.id}"),
                    )
                }
            }
            Text(
                PeopleAccess.spoken(device.code),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.testTag("waiting-code-${device.id}"),
            )
        }
        Text(
            PeopleAccess.approvalPrompt(device),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        val approve = actions.approve
        if (approve != null && access.mayApprove(device, access.thisDevice)) {
            OutlinedButton(onClick = { approve(device) }, modifier = Modifier.testTag("waiting-allow-${device.id}")) { Text("Allow") }
        }
    }
}

@Composable
private fun ThisDeviceAdmin(device: PeopleAccess.Device, shownCode: String?, actions: PeopleAccessActions) {
    SectionTitle("This device")
    Column(Modifier.testTag("people-this-device"), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        when (device.stepUp) {
            StepUpKeyStanding.TRUSTED ->
                Text("Admin actions are on", color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("admin-actions-on"))
            StepUpKeyStanding.PROPOSED -> Waiting(shownCode ?: device.code)
            StepUpKeyStanding.NONE, StepUpKeyStanding.EXPIRED -> {
                val allow = actions.allowAdminActions
                if (shownCode != null) {
                    Waiting(shownCode)
                } else if (allow != null) {
                    OutlinedButton(onClick = allow, modifier = Modifier.testTag("admin-actions-allow")) { Text("Allow admin actions") }
                }
            }
        }
        Text(
            when (device.stepUp) {
                StepUpKeyStanding.TRUSTED -> "Changes to who has access ask for your fingerprint, face or screen lock first."
                StepUpKeyStanding.PROPOSED -> "Open People on another of your devices and allow this one. It checks this code." // casing ok: People is the screen's name
                else -> "Changes to who has access then ask for your fingerprint, face or screen lock on this device."
            },
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun Waiting(code: String) {
    Text("Approve on another device", style = MaterialTheme.typography.bodyMedium)
    Text(
        PeopleAccess.spoken(code),
        style = MaterialTheme.typography.headlineSmall,
        fontWeight = FontWeight.SemiBold,
        modifier = Modifier.testTag("admin-actions-code"),
    )
}
