package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.ceremony.CeremonyRunner
import com.farcooler.ceremony.CeremonyStore
import com.farcooler.ceremony.CodeImage
import com.farcooler.ceremony.CodeScanner
import com.farcooler.ceremony.ScanScreen
import com.farcooler.data.Identity
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * This device asking to be added: show a code, then scan the reply.
 *
 * The code carries this device's public key, its name, an opaque account id, the
 * channel and a random ceremony id. Nothing in it is a secret — a photograph of
 * it enrolls keys belonging to a device the photographer does not hold — which is
 * why it can simply sit on a screen.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun JoinScreen(model: AppModel, onBack: () -> Unit) {
    val email by model.account.email.collectAsStateWithLifecycle()
    val userId by model.account.userId.collectAsStateWithLifecycle()

    val store = remember(userId, email) {
        CeremonyStore(
            account = userId,
            accountEmail = email,
            deviceName = CeremonyStore.thisDeviceName(),
        )
    }
    val scanner = remember { CodeScanner() }

    val phase by store.phase.collectAsStateWithLifecycle()
    val code by store.code.collectAsStateWithLifecycle()
    val granted by store.granted.collectAsStateWithLifecycle()
    val somePending by store.someRunnersPending.collectAsStateWithLifecycle()
    val scanned by scanner.scanned.collectAsStateWithLifecycle()

    val signedIn = userId.isNotEmpty()

    val coroutines = rememberCoroutineScope()

    /**
     * Read the device key off the main thread.
     *
     * [Identity.publicKey] derives from the private key, generating one on first
     * use, and takes a lock the app's own launch-time keygen may be holding. That
     * is several hundred milliseconds of nothing on the frame that is trying to
     * draw this screen.
     */
    fun showOffer() {
        coroutines.launch {
            val key = withContext(Dispatchers.IO) { Identity.publicKey }
            store.showOffer(key)
        }
    }

    LaunchedEffect(signedIn) {
        if (signedIn && phase is CeremonyStore.Phase.Idle) showOffer()
    }
    LaunchedEffect(scanned) {
        val payload = scanned ?: return@LaunchedEffect
        store.takeReply(payload)
        // Written the moment the reply is taken, not when someone taps Done: a
        // reply is consumed once, so a back press off the next screen would
        // otherwise throw away the only copy of it.
        if (store.phase.value is CeremonyStore.Phase.Done) {
            adopt(model, store.granted.value)
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Add this device") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        }
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            if (!signedIn) {
                SignInFirst(alsoPaste = true, onDone = onBack)
                return@Column
            }
            when (val current = phase) {
                is CeremonyStore.Phase.Idle, is CeremonyStore.Phase.ShowingOffer -> OfferScreen(
                    code = code,
                    onScan = {
                        scanner.start()
                        store.beginScanning()
                    },
                )

                is CeremonyStore.Phase.Scanning -> ScanScreen(
                    scanner = scanner,
                    instruction = "Point the camera at the code the other device is showing.",
                    onCancel = { store.showCodeAgain() },
                )

                is CeremonyStore.Phase.Done -> AddedScreen(granted, somePending, onDone = onBack)

                is CeremonyStore.Phase.Refused -> RefusalScreen(
                    refusal = current.refusal,
                    retry = "Show a new code",
                    onRetry = { showOffer() },
                    onDone = onBack,
                )

                // The other side of the ceremony. Unreachable here.
                else -> Unit
            }
        }
    }
}

// MARK: - This device's code

@Composable
private fun OfferScreen(code: String, onScan: () -> Unit) {
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text(
            "Show this to a device you’ve already added",
            style = MaterialTheme.typography.titleSmall,
            textAlign = TextAlign.Center,
        )
        CodeImage(code)
        Text(
            "It carries this device’s public key and nothing secret. The other device picks " +
                "which runners to grant, then shows a code back.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Button(onClick = onScan, modifier = Modifier.fillMaxWidth()) {
            Text("Scan the code it shows back")
        }
    }
}

// MARK: - Added

@Composable
private fun AddedScreen(
    granted: List<CeremonyRunner>,
    somePending: Boolean,
    onDone: () -> Unit,
) {
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("These runners were granted", style = MaterialTheme.typography.titleMedium)
        Text(
            "This device can reach each one once its key is on that runner.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        for (runner in granted) {
            Column {
                Text(runner.label, style = MaterialTheme.typography.bodyMedium)
                Text(
                    runner.reach.detail(runner.user) +
                        if (runner.pending) " · not yet" else "",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        // A granted runner this app cannot record is stated, never dropped
        // silently. The alternative to reading it here is meeting it later as a
        // runner that is simply absent from the list, with nothing on any
        // screen having said so.
        val unstorable = granted.filterNot { it.isStorable }
        if (unstorable.isNotEmpty()) {
            val names = unstorable.joinToString(", ") {
                it.label.ifEmpty { it.reach.name(it.user) }
            }
            Text(
                if (unstorable.size == 1) {
                    "$names is reachable only through a tunnel, which this device can’t use yet."
                } else {
                    "$names are reachable only through a tunnel, which this device can’t " +
                        "use yet."
                },
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.tertiary,
            )
        }
        if (somePending) {
            Text(
                "The ones marked “not yet” hadn’t taken this device’s key when it was granted. " +
                    "They’ll take it when the other device next reaches them.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.tertiary,
            )
        }
        Spacer(Modifier.height(8.dp))
        Button(onClick = onDone, modifier = Modifier.fillMaxWidth()) { Text("Done") }
    }
}

/**
 * Write the granted runners into this device's own list.
 *
 * Matched by where they are reached and as whom, rather than by the id in the
 * manifest: two devices generate their own ids for the same runner, so adopting
 * by id would leave someone with the same box listed twice.
 *
 * A tunneled runner is not written in at all: [com.farcooler.data.Runner]
 * records an address and a port, and a tunnel has neither. `AddedScreen` names
 * it, so nobody has to notice a runner that quietly did not arrive.
 */
private fun adopt(model: AppModel, granted: List<CeremonyRunner>) {
    for (entry in granted) {
        val arriving = entry.asRunner() ?: continue
        val existing = model.hosts.hosts.value.firstOrNull {
            it.address == arriving.address && it.user == arriving.user &&
                it.port == arriving.port
        }
        if (existing == null) {
            model.hosts.add(arriving)
            continue
        }
        // Already known. The pin is the one thing worth taking from the
        // manifest, and only when this device has none of its own: a fingerprint
        // someone approved here outranks one that arrived in a code.
        if (existing.fingerprint == null && entry.hostKey.isNotEmpty()) {
            model.hosts.update(existing.copy(fingerprint = entry.hostKey))
        }
    }
}
