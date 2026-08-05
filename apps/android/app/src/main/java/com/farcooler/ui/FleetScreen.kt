package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.PanTool
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.automirrored.outlined.HelpOutline
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material.icons.outlined.PauseCircle
import androidx.compose.material.icons.outlined.Pending
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalDrawerSheet
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.model.AgentActivity
import com.farcooler.model.StateKind
import com.farcooler.model.Terminal
import com.farcooler.model.Workspace
import com.farcooler.net.Connection
import com.farcooler.net.FleetEntry
import com.farcooler.net.TerminalRef
import kotlinx.coroutines.launch

/**
 * Every worktree on every machine, in one scroll area.
 *
 * Shown two places — as the whole screen when nothing is running anywhere, and
 * inside the drawer over a terminal — so a task started from either one works
 * the same way and neither loses a capability the other has.
 *
 * Every state here is DERIVED by the daemon at the moment of asking. This
 * screen never computes a terminal's state, because a client that re-derives
 * can disagree with the daemon and with the Mac about the same terminal.
 */
@Composable
fun FleetDrawer(
    model: AppModel,
    onSelect: (TerminalRef) -> Unit,
    onSettings: () -> Unit,
    onAuthorize: () -> Unit,
) {
    ModalDrawerSheet {
        Column(Modifier.fillMaxSize()) {
            FleetBody(
                model = model,
                onSelect = onSelect,
                modifier = Modifier.weight(1f),
                contentPadding = PaddingValues(bottom = 8.dp),
            )
            HorizontalDivider()
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TextButton(onClick = onSettings) {
                    Icon(Icons.Outlined.Settings, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                    Text("This device")
                }
                Spacer(Modifier.weight(1f))
                TextButton(onClick = onAuthorize) { Text("Authorize") }
            }
        }
    }
}

/** The fleet as a whole screen, for a fleet with nothing running to land on. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FleetScreen(model: AppModel, onSelect: (TerminalRef) -> Unit, onOpenDrawer: () -> Unit) {
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Worktrees") },
                navigationIcon = {
                    IconButton(onClick = onOpenDrawer) {
                        Icon(Icons.Outlined.Menu, contentDescription = "Show the fleet")
                    }
                },
            )
        }
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    model.fleet.refreshAll()
                    refreshing = false
                }
            },
            modifier = Modifier.padding(padding),
        ) {
            FleetBody(model = model, onSelect = onSelect, modifier = Modifier.fillMaxSize())
        }
    }
}

@Composable
private fun FleetBody(
    model: AppModel,
    onSelect: (TerminalRef) -> Unit,
    modifier: Modifier = Modifier,
    contentPadding: PaddingValues = PaddingValues(0.dp),
) {
    val entries by model.fleet.entries.collectAsStateWithLifecycle()
    val connections by model.fleet.active.collectAsStateWithLifecycle()
    val hosts by model.hosts.hosts.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()

    var showQuickTask by remember { mutableStateOf(false) }
    var showNewWorkspace by remember { mutableStateOf(false) }
    var showHidden by remember { mutableStateOf(false) }
    var editingHost by remember { mutableStateOf<com.farcooler.data.Host?>(null) }
    var addingHost by remember { mutableStateOf(false) }
    var newTerminalIn by remember { mutableStateOf<Pair<Connection, Workspace>?>(null) }

    // Naming the machine only earns its place once there is more than one.
    // With a single machine connected its name is on every row and says
    // nothing about which row is which.
    val namesMachines = connections.size > 1

    val visible = entries.filter { showHidden || !it.workspace.isHidden }
    val hiddenCount = entries.count { it.workspace.isHidden }

    LazyColumn(modifier = modifier, contentPadding = contentPadding) {
        item {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // Sparkles for "describe it", a plain plus for "fill in the
                // form" — the same two flows the Mac keeps side by side, kept
                // apart here by icon rather than by picking a winner.
                TextButton(onClick = { showQuickTask = true }) {
                    Icon(Icons.Filled.AutoAwesome, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Quick task")
                }
                Spacer(Modifier.weight(1f))
                IconButton(onClick = { showNewWorkspace = true }) {
                    Icon(Icons.Filled.Add, contentDescription = "New worktree")
                }
            }
        }

        // A machine that failed says so where its rows would be, rather than
        // dropping out of the list. Its rows are still there when it had any:
        // reads keep showing the last good fetch.
        items(connections, key = { it.host.id }) { connection ->
            MachineStatusRow(
                connection = connection,
                showLabel = namesMachines,
                onRetry = { model.fleet.retry(connection.host.id) },
                // Not `retry`: this machine already has a session object with a
                // backoff armed, and starting over would discard the schedule
                // rather than skip the wait it is counting down.
                onReconnectNow = { connection.reconnectNow() },
                onTrust = { fingerprint ->
                    model.hosts.trust(connection.host, fingerprint)
                    model.fleet.retry(
                        connection.host.id,
                        connection.host.copy(fingerprint = fingerprint),
                    )
                },
                onReviewKey = {
                    model.hosts.forgetKey(connection.host)
                    model.fleet.retry(connection.host.id, connection.host.copy(fingerprint = null))
                },
                onEdit = { editingHost = connection.host },
            )
        }

        if (visible.isEmpty()) {
            item {
                Text(
                    if (entries.isEmpty()) "No worktrees on any connected machine."
                    else "Every worktree is hidden.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                )
            }
        }

        for (entry in visible) {
            item(key = "${entry.host.id}/${entry.workspace.id}") {
                WorkspaceHeader(
                    entry = entry,
                    showMachine = namesMachines,
                    onHide = { hidden ->
                        scope.launch { entry.connection.setHidden(entry.workspace, hidden) }
                    },
                    onNewTerminal = { newTerminalIn = entry.connection to entry.workspace },
                )
            }
            // Creation order, always. Sorting whatever needs you to the top
            // read well until you watched it happen: an agent three rows down
            // finishes, every row under it slides, and the tap you had already
            // committed to lands on something else. Attention is a mark on a
            // row, and a mark you can find in a list that holds still beats one
            // that comes to you by moving the list.
            val numbering = entry.workspace.ordinals()
            items(entry.workspace.terminals, key = { "${entry.host.id}/${it.id}" }) { terminal ->
                TerminalRow(
                    terminal = terminal,
                    ordinal = numbering[terminal.id],
                    onClick = {
                        onSelect(TerminalRef(entry.host.id, entry.workspace.id, terminal.id))
                    },
                    onAction = { action ->
                        scope.launch { entry.connection.act(action, terminal) }
                    },
                )
            }
            if (entry.workspace.terminals.isEmpty()) {
                item(key = "${entry.host.id}/${entry.workspace.id}/empty") {
                    Text(
                        "No terminals",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(start = 32.dp, top = 2.dp, bottom = 8.dp),
                    )
                }
            }
        }

        if (hiddenCount > 0) {
            item {
                TextButton(
                    onClick = { showHidden = !showHidden },
                    modifier = Modifier.padding(horizontal = 8.dp),
                ) {
                    Icon(Icons.Outlined.VisibilityOff, null, Modifier.size(16.dp))
                    Spacer(Modifier.width(6.dp))
                    Text(if (showHidden) "Hide hidden worktrees" else "$hiddenCount hidden")
                }
            }
        }

        item {
            HorizontalDivider(Modifier.padding(vertical = 4.dp))
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    Icons.Outlined.Dns,
                    null,
                    Modifier.size(16.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.width(8.dp))
                Text(
                    liveSummary(connections),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.weight(1f))
                TextButton(onClick = { addingHost = true }) { Text("Add a machine") }
            }
        }
    }

    if (showQuickTask) {
        QuickTaskSheet(model = model, onDismiss = { showQuickTask = false })
    }
    if (showNewWorkspace) {
        NewWorkspaceSheet(model = model, onDismiss = { showNewWorkspace = false })
    }
    newTerminalIn?.let { (connection, workspace) ->
        NewTerminalSheet(
            connection = connection,
            workspace = workspace,
            onDismiss = { newTerminalIn = null },
        )
    }
    if (addingHost) {
        HostEditorSheet(
            existing = null,
            onSave = { model.addHost(it) },
            onRemove = null,
            onDismiss = { addingHost = false },
        )
    }
    editingHost?.let { host ->
        HostEditorSheet(
            existing = host,
            onSave = { model.hosts.update(it) },
            onRemove = { model.removeHost(it) },
            onDismiss = { editingHost = null },
        )
    }
}

private fun liveSummary(connections: List<Connection>): String {
    val healthy = connections.count { it.fleet.value.runtimeHealthy }
    val live = connections.sumOf { it.fleet.value.livePanes }
    if (connections.isEmpty()) return "No machines"
    if (healthy == 0) return "tmux unavailable"
    val machines = if (connections.size == 1) "1 machine" else "${connections.size} machines"
    return "$live live · $machines"
}

/**
 * What one machine is doing, when that is not simply "answering".
 *
 * A machine that stops answering keeps its rows, dimmed, rather than dropping
 * them — so this row is what explains why they are stale. Every failure has
 * exactly one useful next move and they are not the same move, which is why
 * this switches on [Connection.Failure] rather than offering "try again" for
 * everything.
 */
@Composable
private fun MachineStatusRow(
    connection: Connection,
    showLabel: Boolean,
    onRetry: () -> Unit,
    onReconnectNow: () -> Unit,
    onTrust: (String) -> Unit,
    onReviewKey: () -> Unit,
    onEdit: () -> Unit,
) {
    val phase by connection.phase.collectAsStateWithLifecycle()
    if (phase is Connection.Phase.Connected) return

    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp)) {
        Text(
            connection.host.displayLabel,
            style = MaterialTheme.typography.titleSmall,
        )
        when (val current = phase) {
            is Connection.Phase.Connecting -> {
                Text(
                    "Connecting…",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            // Not an error, and deliberately not worded as one: the rows above
            // are this machine's last good answer and are still worth reading.
            //
            // The attempt number is left out. "Reconnecting (4)" prices a wait
            // nobody asked for and reads as an error count; what someone wants
            // to know here is whether to keep waiting or tap, and the button
            // beside it answers that.
            is Connection.Phase.Reconnecting -> {
                Text(
                    "Reconnecting…",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Row {
                    TextButton(onClick = onReconnectNow) { Text("Reconnect now") }
                    TextButton(onClick = onEdit) { Text("Edit") }
                }
            }

            is Connection.Phase.NeedsApproval -> {
                Text(
                    "This machine presented a key Far Cooler has never seen:",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    current.fingerprint,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                )
                Text(
                    "Check it on the machine: " +
                        "ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Row {
                    TextButton(onClick = { onTrust(current.fingerprint) }) { Text("Trust it") }
                    TextButton(onClick = onEdit) { Text("Edit") }
                }
            }

            is Connection.Phase.Failed -> {
                val kind = Connection.Failure.of(current.message)
                Text(
                    failureHeadline(kind, connection),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
                Text(
                    failureDetail(kind, connection, current.message),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Row {
                    when (kind) {
                        Connection.Failure.HOST_KEY_CHANGED ->
                            TextButton(onClick = onReviewKey) { Text("Review the new key") }

                        Connection.Failure.KEY_NOT_TRUSTED ->
                            TextButton(onClick = onReviewKey) { Text("Show the key again") }

                        else -> TextButton(onClick = onRetry) { Text("Try again") }
                    }
                    TextButton(onClick = onEdit) { Text("Edit") }
                }
            }

            Connection.Phase.Connected -> Unit
        }
        HorizontalDivider(Modifier.padding(top = 8.dp))
    }
}

private fun failureHeadline(kind: Connection.Failure, connection: Connection): String = when (kind) {
    Connection.Failure.KEY_REJECTED -> "Not authorized yet"
    Connection.Failure.HOST_KEY_CHANGED -> "This machine's key changed"
    Connection.Failure.UNREACHABLE -> "Can't reach ${connection.host.address}"
    Connection.Failure.DAEMON_MISSING -> "Far Cooler isn't installed there"
    Connection.Failure.NO_IDENTITY -> "This device has no key"
    Connection.Failure.KEY_NOT_TRUSTED -> "Key not trusted"
    Connection.Failure.STOPPED -> "Stopped waiting"
    Connection.Failure.OTHER -> "Can't connect"
}

/**
 * Ours wherever we know what happened, the core's own text only where we do
 * not. The raw string crossing up from Rust is written for whoever is reading a
 * log — lowercase, ending in things like "(os error 61)" — and putting that in
 * front of someone who just wants their machine back is asking them to
 * translate. Two cases keep it deliberately: the changed host key, whose
 * message carries the two fingerprints being compared, and the unclassified
 * failure, where the core's account is the only account there is.
 */
private fun failureDetail(
    kind: Connection.Failure,
    connection: Connection,
    message: String,
): String = when (kind) {
    Connection.Failure.KEY_REJECTED ->
        "${connection.host.user}@${connection.host.address} hasn't been given this device's key."

    Connection.Failure.UNREACHABLE ->
        "Nothing answered on port ${connection.host.port}. The machine may be asleep, " +
            "or the address may be wrong."

    Connection.Failure.DAEMON_MISSING ->
        "SSH connected, but the Far Cooler daemon didn't answer. Install it there."

    else -> message
}

@Composable
private fun WorkspaceHeader(
    entry: FleetEntry,
    showMachine: Boolean,
    onHide: (Boolean) -> Unit,
    onNewTerminal: () -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, top = 12.dp, bottom = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                entry.workspace.task.ifBlank { entry.workspace.branch },
                style = MaterialTheme.typography.titleSmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                buildString {
                    append(entry.workspace.branch)
                    if (showMachine) append(" · ${entry.host.displayLabel}")
                },
                style = MaterialTheme.typography.labelSmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Box {
            IconButton(onClick = { menu = true }) {
                Icon(Icons.Filled.MoreVert, contentDescription = "Worktree actions")
            }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                DropdownMenuItem(
                    text = { Text("New terminal…") },
                    onClick = {
                        menu = false
                        onNewTerminal()
                    },
                )
                DropdownMenuItem(
                    text = { Text(if (entry.workspace.isHidden) "Unhide" else "Hide") },
                    onClick = {
                        menu = false
                        onHide(!entry.workspace.isHidden)
                    },
                )
            }
        }
    }
}

@Composable
private fun TerminalRow(
    terminal: Terminal,
    ordinal: Int?,
    onClick: () -> Unit,
    onAction: (Connection.Action) -> Unit,
) {
    val kind = StateKind.parse(terminal.state)
    var menu by remember { mutableStateOf(false) }

    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier
                .size(8.dp)
                .clip(CircleShape)
                .background(processColor(kind))
        )
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    terminal.label,
                    style = MaterialTheme.typography.bodyLarge,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (ordinal != null) {
                    Spacer(Modifier.width(4.dp))
                    Text(
                        "$ordinal",
                        style = MaterialTheme.typography.labelSmall,
                        fontFamily = FontFamily.Monospace,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Text(
                terminal.state.lowercase(),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        // The reason to have opened the app. Only the two states worth acting
        // on get colour, so a list of twenty still reads at a glance.
        if (terminal.agent.isAgent && terminal.agent != AgentActivity.UNKNOWN) {
            Icon(
                activityIcon(terminal.agent),
                contentDescription = terminal.agent.label,
                tint = attentionColor(terminal.agent),
                modifier = Modifier.size(if (terminal.agent.wantsAttention) 20.dp else 16.dp),
            )
        }

        Box {
            IconButton(onClick = { menu = true }) {
                Icon(Icons.Filled.MoreVert, contentDescription = "Terminal actions")
            }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                if (kind == StateKind.LOST) {
                    DropdownMenuItem(
                        text = { Text("Dismiss") },
                        onClick = {
                            menu = false
                            onAction(Connection.Action.DISMISS_LOST)
                        },
                    )
                }
                DropdownMenuItem(
                    text = { Text("Restart") },
                    onClick = {
                        menu = false
                        onAction(Connection.Action.RESTART)
                    },
                )
                if (kind == StateKind.RUNNING || kind == StateKind.STARTING) {
                    DropdownMenuItem(
                        text = { Text("Stop") },
                        onClick = {
                            menu = false
                            onAction(Connection.Action.STOP)
                        },
                    )
                }
            }
        }
    }
}

/** The same vocabulary the Apple apps use, in this platform's icon set. */
fun activityIcon(activity: AgentActivity): ImageVector = when (activity) {
    AgentActivity.NONE -> Icons.Outlined.Terminal
    AgentActivity.IDLE -> Icons.Outlined.PauseCircle
    AgentActivity.WORKING -> Icons.Outlined.Pending
    AgentActivity.BLOCKED -> Icons.Filled.PanTool
    AgentActivity.DONE -> Icons.Filled.CheckCircle
    AgentActivity.UNKNOWN -> Icons.AutoMirrored.Outlined.HelpOutline
}
