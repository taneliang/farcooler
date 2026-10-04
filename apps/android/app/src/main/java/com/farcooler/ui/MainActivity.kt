package com.farcooler.ui

import android.Manifest
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.LaunchedEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.model.DestinationPayloads
import com.farcooler.model.FirstRunCopy
import com.farcooler.model.NotificationAsk
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val model: AppModel by viewModels()

    /**
     * Asked for at launch once there is a runner, and before that when the
     * first one arrives, behind [NotificationExplainer] (ov-205).
     *
     * Not on the first notification: the point of this feature is being told
     * about an agent while you are not looking at the app, and a permission
     * prompt that only appears once you ARE looking has already missed it.
     * Not before the first runner either: the system's dialog then covered the
     * first screen, before the person had seen what the app was for, and a
     * refusal at that moment is permanent.
     */
    private val askForNotifications =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Drawn behind the system bars. A terminal is the content, and letting
        // it run under a translucent status bar is what makes a phone-sized
        // grid worth having — it is several more columns.
        enableEdgeToEdge()

        if (NotificationAsk.asksAtLaunch(hasRunners = model.hosts.hosts.value.isNotEmpty())) {
            askForNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        // Not again when the activity is only being rebuilt (a rotation, or a
        // process restored from saved state): the intent that launched it is
        // handed back, and a notification tapped an hour ago would pull the
        // phone back to its subject (ov-183).
        if (savedInstanceState == null) handleIntent(intent)

        setContent {
            FarCoolerTheme {
                RootScreen(model)
                NotificationExplainer(model) { askForNotifications.launch(Manifest.permission.POST_NOTIFICATIONS) }
            }
        }

        lifecycleScope.launch {
            // Polling belongs to the foreground. A poll is an SSH round trip
            // and a radio wake-up, and while the app is backgrounded nothing is
            // reading the answer — the push path is what covers a phone in a
            // pocket.
            repeatOnLifecycle(Lifecycle.State.RESUMED) {
                model.setForeground(true)
                try {
                    kotlinx.coroutines.awaitCancellation()
                } finally {
                    model.setForeground(false)
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    /**
     * Two things arrive this way and nothing else does: a tapped notification,
     * which names a terminal, and the sign-in redirect.
     */
    private fun handleIntent(intent: Intent) {
        intent.data?.let { uri ->
            lifecycleScope.launch { model.account.handleCallback(uri) }
        }
        // Which extras a tap carries, and why an empty terminal is absent, is
        // [DestinationPayloads]'s to say. What it names opens as a launch from
        // a notification: held for its runner, then opened or dropped.
        DestinationPayloads.from({ intent.getStringExtra(it) })?.let(model::open)
    }
}

/**
 * One line on what the permission is for, shown when the first runner is
 * added and before the system asks (ov-205). Allow asks; Not now doesn't.
 */
@Composable
private fun NotificationExplainer(model: AppModel, onAllow: () -> Unit) {
    val hosts by model.hosts.hosts.collectAsStateWithLifecycle()
    var hadRunners by remember { mutableStateOf(hosts.isNotEmpty()) }
    var explaining by remember { mutableStateOf(false) }
    val hasRunners = hosts.isNotEmpty()
    LaunchedEffect(hasRunners) {
        if (NotificationAsk.explainsAfterFirstRunner(hadRunners, hasRunners)) explaining = true
        hadRunners = hasRunners
    }
    if (explaining) {
        AlertDialog(
            onDismissRequest = { explaining = false },
            title = { Text(FirstRunCopy.NOTIFY_TITLE) },
            text = { Text(FirstRunCopy.NOTIFY_BODY) },
            confirmButton = {
                TextButton(onClick = {
                    explaining = false
                    onAllow()
                }) { Text(FirstRunCopy.NOTIFY_ALLOW) }
            },
            dismissButton = { TextButton(onClick = { explaining = false }) { Text(FirstRunCopy.NOTIFY_DECLINE) } },
        )
    }
}
