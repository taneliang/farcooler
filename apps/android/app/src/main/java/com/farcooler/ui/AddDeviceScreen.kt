package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.ceremony.CeremonyStore
import com.farcooler.ceremony.CodeImage
import com.farcooler.ceremony.CodeScanner
import com.farcooler.ceremony.ConfirmingTap
import com.farcooler.ceremony.Enroller
import com.farcooler.ceremony.Refusal
import com.farcooler.ceremony.ScanScreen
import kotlinx.coroutines.launch

/**
 * Adding a device from one you already trust: scan its code, pick the runners it
 * may reach, confirm, and show the reply back.
 *
 * The ORDER of the screens is the security argument. A code from another account
 * reaches the mismatch screen and stops there — the runner list is not behind
 * it, because a list of addresses on screen with only a fingerprint between a
 * stranger and them is the thing this flow exists to avoid. That account rule is
 * Rust's, in `ceremony::accept_offer`; this screen only knows which word came
 * back.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AddDeviceScreen(model: AppModel, onBack: () -> Unit) {
    val context = LocalContext.current
    val coroutines = rememberCoroutineScope()

    val email by model.account.email.collectAsStateWithLifecycle()
    val userId by model.account.userId.collectAsStateWithLifecycle()
    val runners by model.hosts.hosts.collectAsStateWithLifecycle()

    val store = remember(userId, email) {
        CeremonyStore(
            account = userId,
            accountEmail = email,
            deviceName = CeremonyStore.thisDeviceName(),
        )
    }
    val scanner = remember { CodeScanner() }

    val phase by store.phase.collectAsStateWithLifecycle()
    val rows by store.rows.collectAsStateWithLifecycle()
    val offer by store.offer.collectAsStateWithLifecycle()
    val fingerprint by store.fingerprint.collectAsStateWithLifecycle()
    val code by store.code.collectAsStateWithLifecycle()
    val declined by store.declined.collectAsStateWithLifecycle()
    val somePending by store.someRunnersPending.collectAsStateWithLifecycle()
    val scanned by scanner.scanned.collectAsStateWithLifecycle()

    /**
     * Asking each runner to write the line. The write itself is the daemon's —
     * see [Enroller] — and a runner this device cannot reach right now is
     * reported as pending rather than as a failure.
     */
    val enroller = remember(model) {
        Enroller { publicKey, label, clientId, granted ->
            granted.filterNot { entry ->
                // Matched by id because these rows came from this device's own
                // runner list a moment ago, so the id in the manifest IS the id
                // of the connection that reaches it.
                model.fleet.connection(entry.id)?.enroll(publicKey, label, clientId) == true
            }.map { it.id }.toSet()
        }
    }

    LaunchedEffect(Unit) {
        if (phase is CeremonyStore.Phase.Idle) store.beginScanning()
    }
    LaunchedEffect(scanned) {
        scanned?.let { store.read(it, runners, model.hosts.selected) }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Add a device") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        }
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding)) {
            // Unreachable from Settings, which only offers this while signed in,
            // and guarded anyway: with no account the core would answer
            // `wrong_account` for every code, and the mismatch screen would name
            // an account nobody is signed into.
            if (userId.isEmpty()) {
                SignInFirst(alsoPaste = false, onDone = onBack)
                return@Column
            }
            when (val current = phase) {
                is CeremonyStore.Phase.Idle, is CeremonyStore.Phase.Scanning ->
                    ScanScreen(
                        scanner = scanner,
                        instruction = "Point the camera at the code on the device you’re adding.",
                        onCancel = onBack,
                    )

                is CeremonyStore.Phase.Mismatch -> MismatchScreen(email, onDone = onBack)

                is CeremonyStore.Phase.Confirming -> ConfirmScreen(
                    deviceName = offer?.name.orEmpty(),
                    accountEmail = email,
                    fingerprint = fingerprint,
                    publicKey = offer?.keyA.orEmpty(),
                    rows = rows,
                    onToggle = { store.toggle(it) },
                    onConfirm = {
                        coroutines.launch {
                            store.confirm(
                                gate = { title, subtitle ->
                                    ConfirmingTap.ask(context, title, subtitle)
                                },
                                enroller = enroller,
                            )
                        }
                    },
                    onCancel = onBack,
                )

                is CeremonyStore.Phase.Enrolling -> Waiting()

                is CeremonyStore.Phase.ShowingManifest -> ReplyScreen(
                    deviceName = offer?.name.orEmpty(),
                    code = code,
                    somePending = somePending,
                    onDone = onBack,
                )

                is CeremonyStore.Phase.Refused -> RefusalScreen(
                    refusal = current.refusal,
                    retry = "Scan again",
                    onRetry = {
                        scanner.start()
                        store.beginScanning()
                    },
                    onDone = onBack,
                )

                // Nothing on this side of the ceremony produces either, and a
                // screen that cannot be reached is better left empty than filled
                // with something invented for it.
                is CeremonyStore.Phase.Done, is CeremonyStore.Phase.ShowingOffer -> Waiting()
            }
        }
    }

    // A declined gate stops here and says so. It does not fall back to adding
    // the device, and it does not explain what the gate wanted — the sentence a
    // person needs is that nothing happened.
    if (declined) {
        AlertDialog(
            onDismissRequest = { store.dismissDeclined() },
            title = { Text("Far Cooler couldn’t confirm it’s you") },
            text = { Text("Nothing was added.") },
            confirmButton = {
                TextButton(onClick = { store.dismissDeclined() }) { Text("OK") }
            },
        )
    }
}

// MARK: - Signed out

/**
 * Shared by both sides of the ceremony, because the reason is the same one: an
 * enrollment is recorded under an account, and a device with no account has
 * nothing to record it under. [alsoPaste] names the way out that only the device
 * being added has.
 */
@Composable
internal fun SignInFirst(alsoPaste: Boolean, onDone: () -> Unit) {
    Column(
        Modifier.fillMaxSize().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text("Sign in first", style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(12.dp))
        Text(
            "Far Cooler records each enrollment under your account, so both devices have to be " +
                "signed into the same one." +
                if (alsoPaste) " You can also add this device by pasting its key." else "",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(28.dp))
        TextButton(onClick = onDone) { Text("Done") }
    }
}

// MARK: - That device is signed into a different account

/**
 * Before the runner list, never after it.
 *
 * It must be impossible to reach the runner list with a mismatched code —
 * otherwise the runners are on screen with only a fingerprint between a stranger
 * and them.
 */
@Composable
private fun MismatchScreen(accountEmail: String, onDone: () -> Unit) {
    Column(
        Modifier.fillMaxSize().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text(
            "That device is signed into a different account",
            style = MaterialTheme.typography.titleMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(12.dp))
        Text(
            "Far Cooler can only add devices signed into $accountEmail. Sign in to that " +
                "account on the new device, then show its code again.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(28.dp))
        Button(onClick = onDone, modifier = Modifier.fillMaxWidth()) { Text("Done") }
    }
}

// MARK: - The confirmation

@Composable
private fun ConfirmScreen(
    deviceName: String,
    accountEmail: String,
    fingerprint: String?,
    publicKey: String,
    rows: List<com.farcooler.ceremony.RunnerRow>,
    onToggle: (String) -> Unit,
    onConfirm: () -> Unit,
    onCancel: () -> Unit,
) {
    val named = deviceName.ifBlank { "That device" }
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text(
            "Add “${deviceName.ifBlank { "this device" }}” to $accountEmail?",
            style = MaterialTheme.typography.titleMedium,
        )
        Text(
            "$named will be able to run agents and commands on the runners you pick, as you. " +
                "Each enrollment is recorded under $accountEmail.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        fingerprint?.let { FingerprintRow(it, publicKey) }

        Column {
            for (row in rows) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clickable { onToggle(row.runner.id) }
                        .padding(vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Checkbox(checked = row.picked, onCheckedChange = { onToggle(row.runner.id) })
                    Column(Modifier.weight(1f)) {
                        Text(row.label, style = MaterialTheme.typography.bodyMedium)
                        Text(
                            row.detail,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }

        Text(
            "Far Cooler adds this key to ~/.ssh/authorized_keys on each runner you pick, and " +
                "changes nothing else. You can add or remove runners later in Settings › Devices.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Button(
            onClick = onConfirm,
            enabled = rows.any { it.picked },
            modifier = Modifier.fillMaxWidth(),
        ) { Text("Add device") }
        TextButton(onClick = onCancel, modifier = Modifier.fillMaxWidth()) { Text("Cancel") }
    }
}

/**
 * The key being granted, short enough to read out and expandable to the whole
 * thing.
 *
 * The short form is what two people compare across a desk; the full key is what
 * somebody pastes into a bug report. Neither is a decision — the target check
 * that matters is made in Rust, against a fingerprint it computes itself.
 */
@Composable
private fun FingerprintRow(fingerprint: String, publicKey: String) {
    var expanded by remember { mutableStateOf(false) }
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .clickable { expanded = !expanded }
            .padding(12.dp),
    ) {
        Text(
            if (expanded) publicKey.ifBlank { fingerprint }
            else CeremonyStore.abbreviated(fingerprint),
            style = MaterialTheme.typography.bodySmall,
            fontFamily = FontFamily.Monospace,
        )
    }
}

@Composable
private fun Waiting() {
    Column(
        Modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        CircularProgressIndicator(Modifier.size(22.dp), strokeWidth = 2.dp)
        Spacer(Modifier.height(12.dp))
        Text(
            "Adding…",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

// MARK: - The reply

@Composable
private fun ReplyScreen(
    deviceName: String,
    code: String,
    somePending: Boolean,
    onDone: () -> Unit,
) {
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text(
            "Scan this with ${deviceName.ifBlank { "the new device" }}",
            style = MaterialTheme.typography.titleSmall,
            textAlign = TextAlign.Center,
        )
        CodeImage(code)
        Text(
            "It carries the runners you picked and how to reach them. Nothing in it is a secret.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        if (somePending) {
            // No cause given, because from here the cause is not knowable.
            Text(
                "Some runners didn’t take the key. They’ll be added when this device next " +
                    "reaches them.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.tertiary,
                textAlign = TextAlign.Center,
            )
        }
        Button(onClick = onDone, modifier = Modifier.fillMaxWidth()) { Text("Done") }
    }
}

/**
 * A refusal, said in the app's own words.
 *
 * Every sentence here comes from [Refusal], which maps the core's stable word to
 * copy. No Rust error string reaches this screen, and nothing on it suggests
 * loosening anything.
 */
@Composable
fun RefusalScreen(
    refusal: Refusal,
    retry: String?,
    onRetry: () -> Unit,
    onDone: () -> Unit,
) {
    Column(
        Modifier.fillMaxSize().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text(
            refusal.title,
            style = MaterialTheme.typography.titleMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(12.dp))
        Text(
            refusal.message,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(28.dp))
        if (retry != null) {
            Button(onClick = onRetry, modifier = Modifier.fillMaxWidth()) { Text(retry) }
            Spacer(Modifier.height(8.dp))
        }
        TextButton(onClick = onDone) { Text("Done") }
    }
}
