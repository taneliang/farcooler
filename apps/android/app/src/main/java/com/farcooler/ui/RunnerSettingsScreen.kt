package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import com.farcooler.data.Theme
import com.farcooler.data.Themes
import com.farcooler.model.AdapterInfo
import com.farcooler.model.AdapterTestOutcome
import com.farcooler.model.HostHealth
import com.farcooler.net.Connection
import kotlinx.coroutines.launch

/**
 * One runner's `config.toml`, from a phone.
 *
 * The reason this belongs on a phone: the runner holding the file is frequently
 * a Linux box with no Far Cooler app on it, reached over ssh. Changing its branch
 * prefix used to mean an ssh session and a text editor, which is not a thing
 * anybody does from a phone — and the theme section's own help text used to say
 * to go and do exactly that.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RunnerSettingsScreen(connection: Connection, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()

    var prefix by remember { mutableStateOf("") }
    var storedPrefix by remember { mutableStateOf("") }
    var themes by remember { mutableStateOf<List<Theme>>(emptyList()) }
    var adapters by remember { mutableStateOf<List<AdapterInfo>>(emptyList()) }
    var health by remember { mutableStateOf<HostHealth?>(null) }
    var loading by remember { mutableStateOf(true) }
    var failure by remember { mutableStateOf<String?>(null) }

    var editingTheme by remember { mutableStateOf<Theme?>(null) }
    var editingAdapter by remember { mutableStateOf<AdapterInfo?>(null) }
    var adapterIsNew by remember { mutableStateOf(false) }

    suspend fun reload() {
        storedPrefix = connection.branchPrefix.value
        prefix = storedPrefix
        // First, because it is what says whether the rest of the screen can be
        // trusted — and because it is `Scope::Read`, unlike everything under it.
        // A phone the runner has given read scope gets a health section and
        // three empty ones, which is a truer picture than a screen that fails
        // whole.
        health = connection.health()
        themes = connection.hostThemes()
        adapters = connection.adapters()
        loading = false
    }

    LaunchedEffect(connection) { reload() }

    // The editors take the whole screen rather than a bottom sheet: nineteen
    // colour rows and seven fields do not fit one, and a sheet that scrolls
    // behind the keyboard is worse than a screen that does not have to.
    editingTheme?.let { theme ->
        ThemeEditorScreen(
            theme = theme,
            onCancel = { editingTheme = null },
            onSave = { edited ->
                editingTheme = null
                scope.launch {
                    connection.upsertTheme(edited)?.let { themes = it }
                    // Every picker reads the merged list, so a saved theme has
                    // to reach it — otherwise the thing you just made is
                    // missing from the one place you would choose it.
                    connection.reloadThemes()
                }
            })
        return
    }
    editingAdapter?.let { adapter ->
        AdapterEditorScreen(
            adapter = adapter,
            isNew = adapterIsNew,
            onTest = { connection.testAdapter(it) },
            onCancel = { editingAdapter = null },
            onSave = { edited ->
                editingAdapter = null
                scope.launch { connection.upsertAdapter(edited)?.let { adapters = it } }
            })
        return
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(connection.host.displayLabel) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    if (loading) {
                        CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    }
                })
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            // What this runner is and whether it is well.
            //
            // Absent entirely on a runner too old to answer, rather than drawn
            // as zeroes — a version of "unknown" and a version of "0.0.0" are
            // not the same claim. iOS's `healthSection` does the same with the
            // same nil.
            health?.let { HealthSection(it) }

            SectionTitle("Branches")
            OutlinedTextField(
                value = prefix,
                onValueChange = { prefix = it },
                label = { Text("Branch prefix") },
                placeholder = { Text("feat/") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Text(
                "Goes in front of a branch name made from a task description. " +
                    "Leave it empty for no prefix.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (prefix != storedPrefix) {
                Button(onClick = {
                    scope.launch {
                        val stored = connection.setBranchPrefix(prefix)
                        if (stored == null) {
                            failure = "That runner didn’t accept the change."
                        } else {
                            storedPrefix = stored
                            prefix = stored
                            failure = null
                        }
                    }
                }) { Text("Save") }
            }

            HorizontalDivider()
            SectionTitle("Themes on this runner")
            Text(
                "Only themes this runner defines are listed. Shipped themes have nothing to " +
                    "delete; saving one under a shipped name overrides it here.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            for (theme in themes) {
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    ThemeStrip(theme)
                    Text(theme.name, Modifier.weight(1f))
                    TextButton(onClick = { editingTheme = theme }) { Text("Edit") }
                    TextButton(onClick = {
                        scope.launch { connection.deleteTheme(theme.name)?.let { themes = it }
                            connection.reloadThemes() }
                    }) { Text("Delete") }
                }
            }
            OutlinedButton(onClick = {
                val taken = themes.map { it.name }.toSet()
                var name = "${Themes.current.name} Copy"
                var n = 2
                while (taken.contains(name)) {
                    name = "${Themes.current.name} Copy $n"
                    n += 1
                }
                editingTheme = Themes.current.copy(name = name)
            }) { Text("Duplicate the current theme") }

            HorizontalDivider()
            SectionTitle("Agents")
            Text(
                "An adapter lets Far Cooler show an agent as a chat instead of its terminal.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            for (adapter in adapters) {
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Column(Modifier.weight(1f)) {
                        Text(adapter.preset)
                        Text(
                            adapter.subtitle(),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                        )
                    }
                    TextButton(onClick = {
                        adapterIsNew = false
                        editingAdapter = adapter
                    }) { Text("Edit") }
                    // Only what the file owns can be removed. A built-in has no
                    // table to delete, so offering it would do nothing.
                    if (adapter.origin == "override" || adapter.origin == "user") {
                        TextButton(onClick = {
                            scope.launch {
                                connection.deleteAdapter(adapter.preset)?.let { adapters = it }
                            }
                        }) { Text(if (adapter.origin == "override") "Revert" else "Delete") }
                    }
                }
            }
            OutlinedButton(onClick = {
                val taken = adapters.map { it.preset }.toSet()
                var name = "my-agent"
                var n = 2
                while (taken.contains(name)) {
                    name = "my-agent-$n"
                    n += 1
                }
                adapterIsNew = true
                editingAdapter = AdapterInfo(preset = name)
            }) { Text("Add an agent") }

            failure?.let {
                HorizontalDivider()
                Text(
                    it,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
        }
    }
}

/**
 * Whether this runner says it is well, and what it is.
 *
 * **[HostHealth.reasons] is the reason this section exists.** Everything else in
 * it is on the phone already — the fleet poll carries the same healthy flag and
 * the same live count, and `loadDaemonBuild` caches the same platform and
 * version — but nothing carried the daemon's account of WHY, and the fleet
 * footer's three words ("tmux unavailable") are the whole of what this app could
 * previously say about a degraded runner. They are also fleet-wide: that footer
 * only colours when EVERY runner is down, so one bad runner out of three said
 * nothing at all anywhere. This screen is per-runner by construction.
 *
 * The reasons are printed one to a line, unsummarized and in the runner's own
 * voice. Summarizing them would throw away the only part that says what to do,
 * and rewriting them would make a machine's words read as Far Cooler's — the
 * split `Trouble` exists for.
 */
@Composable
private fun HealthSection(health: HostHealth) {
    // Its own Column, tighter than the screen's 12dp. Everything in here is one
    // paragraph about one runner; at the screen's spacing the eight lines read
    // as eight separate settings.
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        SectionTitle("This runner")
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                if (health.healthy) Icons.Outlined.CheckCircle else Icons.Outlined.Warning,
                contentDescription = null,
                // Red for degraded, not amber: amber on this phone means an agent
                // is waiting on you and nothing else. `7e4a4f7` settled that for
                // iOS and `FleetScreen`'s footer already follows it.
                tint = if (health.healthy) MaterialTheme.colorScheme.onSurfaceVariant
                else MaterialTheme.colorScheme.error,
                modifier = Modifier.size(18.dp),
            )
            Spacer(Modifier.width(8.dp))
            Text(if (health.healthy) "Healthy" else "Degraded")
        }
        for (reason in health.reasons) {
            DetailBox(reason)
        }
        HealthFact("Far Cooler", health.daemonVersion.ifBlank { "unknown" })
        HealthFact("Platform", health.platform.ifBlank { "unknown" })
        HealthFact("Live panes", health.livePanes.toString())
        // The one fact here that is not also somewhere else in the app, apart from
        // the reasons. Small and last, because it is what you read when somebody
        // asks for it and never otherwise.
        if (health.protocolVersion > 0) {
            Text(
                "Protocol ${health.protocolVersion}.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        HorizontalDivider(Modifier.padding(top = 6.dp))
    }
}

/** A label and its value, for the health section's short list of facts. */
@Composable
private fun HealthFact(label: String, value: String) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(
            label,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.weight(1f))
        Text(value, style = MaterialTheme.typography.bodySmall)
    }
}

/** A theme's colours as one strip, for a list row. */
@Composable
fun ThemeStrip(theme: Theme) {
    Row(horizontalArrangement = Arrangement.spacedBy(1.dp)) {
        // The ground, the text, and the eight normal colours. Enough to tell two
        // themes apart at a glance, which is all a row has to do.
        val preview = listOf(theme.background, theme.foreground) + theme.ansi.take(8)
        for (packed in preview) {
            Box(
                Modifier
                    .width(5.dp)
                    .height(18.dp)
                    // Opaque alpha, since the grid's colours carry none.
                    .background(Color(0xFF000000.toInt() or packed)))
        }
    }
}
