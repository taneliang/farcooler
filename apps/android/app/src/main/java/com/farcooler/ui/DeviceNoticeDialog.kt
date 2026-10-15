package com.farcooler.ui

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.account.DeviceNotice
import kotlinx.coroutines.launch

/**
 * Devices added to or removed from the account since this phone last said so
 * (ov-550 review M1), until the person dismisses it. From the directory's own
 * record, not the push a stolen session can silence.
 */
@Composable
fun DeviceNoticeDialog(model: AppModel) {
    val events by model.deviceNotice.events.collectAsStateWithLifecycle()
    LifecycleResumeEffect(Unit) {
        model.refreshDeviceNotice()
        onPauseOrDispose {}
    }
    if (events.isEmpty()) return
    AlertDialog(
        onDismissRequest = {},
        title = { Text(DeviceNotice.title(events)) },
        text = { Text(DeviceNotice.detail(events)) },
        confirmButton = { TextButton(onClick = { model.deviceNotice.dismiss() }) { Text("Dismiss") } },
    )
}
