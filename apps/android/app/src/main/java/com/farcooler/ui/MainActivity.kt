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
import com.farcooler.notify.Notifier
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val model: AppModel by viewModels()

    /**
     * Asked for at launch, alongside the device key.
     *
     * Not on the first notification: the point of this feature is being told
     * about an agent while you are not looking at the app, and a permission
     * prompt that only appears once you ARE looking has already missed it.
     */
    private val askForNotifications =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Drawn behind the system bars. A terminal is the content, and letting
        // it run under a translucent status bar is what makes a phone-sized
        // grid worth having — it is several more columns.
        enableEdgeToEdge()

        askForNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        handleIntent(intent)

        setContent {
            FarCoolerTheme {
                RootScreen(model)
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
        // Two spellings for one id, because two different things draw the
        // notification. This app's own banner puts it under its own key; a push
        // the app never saw is drawn by Firebase, which copies the message's
        // `data` keys into the launch intent verbatim — so a tapped push
        // arrives under the relay's spelling. Reading only the first is why
        // tapping a notification about an agent on a sleeping phone opened the
        // app on whatever it was showing last. See [Notifier.PUSH_EXTRA_TERMINAL].
        val terminal = intent.getStringExtra(Notifier.EXTRA_TERMINAL)
            ?: intent.getStringExtra(Notifier.PUSH_EXTRA_TERMINAL)
        terminal?.let { model.openByTerminalId(it) }
    }
}
