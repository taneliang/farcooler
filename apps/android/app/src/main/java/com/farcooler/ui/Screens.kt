package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
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
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.account.AppVersion
import com.farcooler.account.Registration
import com.farcooler.account.RegistrationKind
import com.farcooler.account.Registrations
import com.farcooler.data.Identity
import com.farcooler.data.Settings
import com.farcooler.data.TerminalFontChoice
import com.farcooler.data.Themes
import com.farcooler.net.Connection
import kotlinx.coroutines.launch

/**
 * The first-run screen: no runners yet, so there is nothing else to show.
 *
 * One statement and two actions, the more important one loud. The ORDER those
 * steps go in is not a preference: a runner that has never seen this device's
 * key refuses the very first connection, so "add one, then authorize" ends
 * onboarding on a failure screen. Authorizing first costs nothing and makes the
 * first connection the one that succeeds — which is why it is the prominent
 * button and adding a runner is the quiet one.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OnboardingScreen(
    onAdd: (com.farcooler.data.Runner) -> Unit,
    onAuthorize: () -> Unit,
    onSettings: () -> Unit,
    showBack: Boolean,
    onBack: () -> Unit,
) {
    var adding by remember { mutableStateOf(false) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {},
                navigationIcon = {
                    if (showBack) {
                        IconButton(onClick = onBack) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                        }
                    }
                },
                actions = {
                    TextButton(onClick = onSettings) { Text("This device") }
                },
            )
        }
    ) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).padding(horizontal = 32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center,
        ) {
            Icon(
                Icons.Outlined.Dns,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(44.dp),
            )
            Spacer(Modifier.height(22.dp))
            Text("Connect a runner", style = MaterialTheme.typography.headlineSmall)
            Spacer(Modifier.height(8.dp))
            Text(
                "Far Cooler runs coding agents on runners you already reach over SSH. " +
                    "Put this device's key on one, then add its address.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(32.dp))
            Button(onClick = onAuthorize, modifier = Modifier.fillMaxWidth()) {
                Text("Authorize this device")
            }
            Spacer(Modifier.height(12.dp))
            TextButton(onClick = { adding = true }) { Text("Add a runner") }
        }
    }

    if (adding) {
        RunnerEditorSheet(
            existing = null,
            onSave = onAdd,
            onRemove = null,
            onDismiss = { adding = false },
        )
    }
}

/**
 * Shows the public key to install on a runner.
 *
 * The manual path, and it stays. There is a ceremony now — two codes and a
 * camera, through [JoinScreen] — but this is what works when there is no trusted
 * device to scan with, and it is what is left the day every device is lost. Its
 * wording is unchanged apart from "runner".
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AuthorizeScreen(onJoin: () -> Unit, onBack: () -> Unit) {
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    val publicKey = remember { Identity.publicKey ?: "could not generate a key" }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Authorize") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        }
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(20.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            // Above the key, because it is the shorter road: someone holding a
            // device that already has runners never needs to paste anything.
            Button(onClick = onJoin, modifier = Modifier.fillMaxWidth()) {
                Text("Add this device with a code")
            }
            Text(
                "Show a code to a device you’ve already added, and it picks which runners this " +
                    "one may reach. Or add the key by hand:",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text("Add this device's public key to the runner:")
            Mono(publicKey)
            Button(
                onClick = { scope.launch { clipboard.writeText("Far Cooler device key", publicKey) } },
            ) {
                Icon(Icons.Filled.ContentCopy, contentDescription = null, Modifier.size(16.dp))
                Spacer(Modifier.width(8.dp))
                Text("Copy public key")
            }
            Text("On the runner, run:")
            Mono("echo '<paste>' >> ~/.ssh/authorized_keys")
            Text(
                "The private key never leaves this device. Revoke it by deleting that line " +
                    "from authorized_keys.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Identity.lastError?.let {
                Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            }
        }
    }
}

@Composable
private fun Mono(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        fontFamily = FontFamily.Monospace,
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(10.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(12.dp),
    )
}

/** What the terminal looks like, what may interrupt you, and who you are. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    model: AppModel,
    onOpenRunnerSettings: (Connection) -> Unit,
    onBack: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboard.current

    val font by model.settings.font.collectAsStateWithLifecycle()
    val themes by Themes.available.collectAsStateWithLifecycle()
    val selectedTheme by Themes.selected.collectAsStateWithLifecycle()
    val fontSize by model.settings.fontSize.collectAsStateWithLifecycle()
    val onAttention by model.settings.notifyOnAttention.collectAsStateWithLifecycle()
    val onDone by model.settings.notifyOnDone.collectAsStateWithLifecycle()
    val allRunners by model.settings.allRunnersAtOnce.collectAsStateWithLifecycle()
    val reshape by model.settings.reshapePanes.collectAsStateWithLifecycle()
    val email by model.account.email.collectAsStateWithLifecycle()
    val signingIn by model.account.signingIn.collectAsStateWithLifecycle()
    val accountError by model.account.lastError.collectAsStateWithLifecycle()
    val pushError by model.push.lastError.collectAsStateWithLifecycle()
    val pushRegistered by model.push.registered.collectAsStateWithLifecycle()
    val connections by model.fleet.active.collectAsStateWithLifecycle()

    LaunchedEffect(connections) { connections.forEach { it.loadDaemonBuild() } }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("This device") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        }
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            SectionTitle("Account")
            if (model.account.isSignedIn) {
                Text(email, style = MaterialTheme.typography.bodyMedium)
                Row {
                    TextButton(onClick = { model.navigate(Route.Devices) }) {
                        Text("Devices and runners")
                    }
                    TextButton(onClick = { scope.launch { model.account.signOut() } }) {
                        Text("Sign out")
                    }
                }

                // "Settings › Devices", which the confirmation screen names as
                // where a grant can be changed later. It has to be here, or that
                // sentence sends someone looking for a screen that does not
                // exist.
                HorizontalDivider()
                SectionTitle("Devices")
                Button(onClick = { model.navigate(Route.AddDevice) }) { Text("Add a device") }
                Text(
                    "Adding a device shows a code for it to scan, and grants only the runners " +
                        "you pick.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                Text(
                    "Sign in so a runner you own can notify this phone while the app is " +
                        "closed. Everything else works without it.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Button(onClick = { model.account.signIn(context) }, enabled = !signingIn) {
                    Text(if (signingIn) "Opening…" else "Sign in")
                }
            }
            accountError?.let { Warning(it) }
            if (model.account.isSignedIn && !pushRegistered) {
                pushError?.let { Warning(it) }
            }

            HorizontalDivider()
            SectionTitle("Notifications")
            SettingRow("When an agent needs you", onAttention) {
                model.settings.setNotifyOnAttention(it)
            }
            SettingRow("When an agent finishes", onDone, enabled = onAttention) {
                model.settings.setNotifyOnDone(it)
            }
            Text(
                "A working agent never notifies you.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            HorizontalDivider()
            SectionTitle("Runners")
            SettingRow("Connect every runner at once", allRunners) {
                model.settings.setAllRunnersAtOnce(it)
            }
            Text(
                "One fleet across every runner, the way the Mac app works. Turn it off to " +
                    "talk only to the runner you picked, which costs less battery when you " +
                    "have several.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            SettingRow("Reshape panes to this screen", reshape) {
                model.settings.setReshapePanes(it)
            }
            Text(
                "A tmux pane is shared. Reshaping it makes text readable here and narrows it " +
                    "for anyone else watching the same terminal; the shape is handed back when " +
                    "you leave.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            HorizontalDivider()
            SectionTitle("Theme")
            Text(
                "Sets the terminal's colors and the app's.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            // Editing the runner's own file, rather than telling someone to go
            // and edit it. That instruction used to be this section's help text —
            // reasonable advice, and not something anybody does from a phone.
            val live = connections.firstOrNull { it.phase.value is Connection.Phase.Connected }
            if (live != null) {
                OutlinedButton(onClick = { onOpenRunnerSettings(live) }) {
                    Text("Settings on ${live.host.displayLabel}")
                }
            }
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (theme in themes) {
                    TextButton(onClick = { Themes.select(theme.name) }) {
                        Text(
                            theme.name,
                            color =
                                if (theme.name == selectedTheme) MaterialTheme.colorScheme.primary
                                else MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }

            HorizontalDivider()
            SectionTitle("Terminal font")
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (choice in TerminalFontChoice.entries) {
                    TextButton(onClick = { model.settings.setFont(choice) }) {
                        Text(
                            choice.label,
                            color =
                                if (choice == font) MaterialTheme.colorScheme.primary
                                else MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("Size", style = MaterialTheme.typography.bodyMedium)
                Slider(
                    value = fontSize,
                    onValueChange = { model.settings.setFontSize(it) },
                    valueRange = Settings.MIN_FONT_SIZE..Settings.MAX_FONT_SIZE,
                    steps = (Settings.MAX_FONT_SIZE - Settings.MIN_FONT_SIZE).toInt() - 1,
                    modifier = Modifier.weight(1f).padding(horizontal = 12.dp),
                )
                Text("${fontSize.toInt()}", style = MaterialTheme.typography.labelMedium)
            }
            // The glyphs a coding agent actually prints — box drawing and a
            // couple of Nerd Font icons — not lorem ipsum. Lorem ipsum in a
            // monospaced regular weight looks identical whichever font failed
            // to load; this does not.
            Text(
                "┌─  claude ·  main\n│ 12 files changed",
                fontFamily = TerminalFonts.family(font),
                style = MaterialTheme.typography.bodySmall.copy(fontSize = fontSize.sp),
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(8.dp))
                    .background(androidx.compose.ui.graphics.Color(com.farcooler.core.TerminalPalette.BACKGROUND))
                    .padding(12.dp),
            )

            HorizontalDivider()
            SectionTitle("Version")
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Far Cooler ${AppVersion.display}", style = MaterialTheme.typography.bodyMedium)
                    Text(
                        "build ${AppVersion.build} · ${AppVersion.channel}",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                IconButton(onClick = {
                    scope.launch {
                        clipboard.writeText("Far Cooler diagnostics", diagnostics(model, connections))
                    }
                }) {
                    Icon(Icons.Filled.ContentCopy, contentDescription = "Copy version details")
                }
            }
            for (connection in connections) {
                val daemon by connection.daemon.collectAsStateWithLifecycle()
                daemon?.let {
                    Text(
                        "${connection.host.displayLabel}: ${it.version} · ${it.platform}" +
                            if (it.matches) "" else " — built from different source",
                        style = MaterialTheme.typography.labelSmall,
                        color =
                            if (it.matches) MaterialTheme.colorScheme.onSurfaceVariant
                            else MaterialTheme.colorScheme.error,
                    )
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

private fun diagnostics(model: AppModel, connections: List<Connection>): String = buildString {
    appendLine("Far Cooler for Android ${AppVersion.display} (build ${AppVersion.build})")
    appendLine("channel: ${AppVersion.channel}")
    appendLine("device: ${android.os.Build.MANUFACTURER} ${android.os.Build.MODEL}")
    appendLine("android: ${android.os.Build.VERSION.RELEASE} (API ${android.os.Build.VERSION.SDK_INT})")
    appendLine("core loaded: ${com.farcooler.core.coreIsAvailable}")
    for (connection in connections) {
        val daemon = connection.daemon.value
        appendLine(
            "${connection.host.displayLabel}: " +
                (daemon?.let { "${it.version} ${it.platform}" } ?: "not reported")
        )
    }
}

/**
 * A section heading, shared by every settings screen in this app.
 *
 * Was private to this file until the runner-settings screens needed the same
 * heading. Two definitions of one heading is how two screens come to look
 * slightly different for no reason anybody can find later.
 */
@Composable
fun SectionTitle(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(top = 4.dp),
    )
}

@Composable
private fun SettingRow(
    label: String,
    checked: Boolean,
    enabled: Boolean = true,
    onChange: (Boolean) -> Unit,
) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = onChange, enabled = enabled)
    }
}

@Composable
private fun Warning(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.tertiary,
    )
}

/**
 * Everything this account has registered, and how to revoke it.
 *
 * Revoking here rather than on the runner is the case that matters: a laptop
 * you no longer have is exactly the one you cannot run a command on.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DevicesScreen(model: AppModel, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    var registrations by remember { mutableStateOf<Registrations?>(null) }
    var loading by remember { mutableStateOf(true) }

    LaunchedEffect(Unit) {
        registrations = model.account.fetchRegistrations()
        loading = false
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Devices and runners") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        }
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (loading) {
                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                return@Column
            }
            val current = registrations
            if (current == null) {
                Text("Couldn’t reach the relay.")
                return@Column
            }

            SectionTitle("Devices")
            for (device in current.devices) {
                RegistrationRow(device) {
                    scope.launch {
                        model.account.revoke(device, RegistrationKind.DEVICE)
                        registrations = model.account.fetchRegistrations()
                    }
                }
            }
            if (current.devices.isEmpty()) {
                Text(
                    "No devices are registered for notifications.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            HorizontalDivider()
            SectionTitle("Runners")
            for (runner in current.runners) {
                RegistrationRow(runner) {
                    scope.launch {
                        model.account.revoke(runner, RegistrationKind.RUNNER)
                        registrations = model.account.fetchRegistrations()
                    }
                }
            }
            if (current.runners.isEmpty()) {
                Text(
                    "No runners are paired to notify this account.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun RegistrationRow(registration: Registration, onRevoke: () -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(registration.label, style = MaterialTheme.typography.bodyMedium)
            Text(
                listOfNotNull(registration.detail, registration.version).joinToString(" · "),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        TextButton(onClick = onRevoke) { Text("Revoke") }
    }
}
