package com.farcooler.ui

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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.Cancel
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
 * Every workspace on every runner, in one scroll area.
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
                title = { Text("Workspaces") },
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
    var editingRunner by remember { mutableStateOf<com.farcooler.data.Runner?>(null) }
    var addingRunner by remember { mutableStateOf(false) }
    var newTerminalIn by remember { mutableStateOf<Pair<Connection, Workspace>?>(null) }

    // Naming the runner only earns its place once there is more than one.
    // With a single runner connected its name is on every row and says
    // nothing about which row is which.
    val namesRunners = connections.size > 1

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
                    Icon(Icons.Filled.Add, contentDescription = "New workspace")
                }
            }
        }

        // A runner that failed says so where its rows would be, rather than
        // dropping out of the list. Its rows are still there when it had any:
        // reads keep showing the last good fetch.
        items(connections, key = { it.host.id }) { connection ->
            RunnerStatusRow(
                connection = connection,
                showLabel = namesRunners,
                onRetry = { model.fleet.retry(connection.host.id) },
                // Not `retry`: this runner already has a session object with a
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
                onEdit = { editingRunner = connection.host },
            )
        }

        if (visible.isEmpty()) {
            item {
                Text(
                    if (entries.isEmpty()) "No workspaces on any connected runner."
                    else "Every workspace is hidden.",
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
                    showRunner = namesRunners,
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
                    Text(if (showHidden) "Hide hidden workspaces" else "$hiddenCount hidden")
                }
            }
        }

        item {
            HorizontalDivider(Modifier.padding(vertical = 4.dp))
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // "tmux unavailable" was set in exactly the typography and
                // exactly the colour of "3 live · 2 runners" — the one sentence
                // on this screen that means every pane on every runner is
                // unreadable, drawn as though it were a healthy count. Both the
                // Mac and iOS give the same three words a coloured mark; this
                // one already has a mark of its own, so the mark takes the
                // colour rather than a second dot being added beside it.
                //
                // Red rather than the Mac's amber. Amber means an agent is
                // waiting on you, and nobody is waiting here: the runtime every
                // pane lives inside is not answering, which is a failure. iOS
                // settled that in `7e4a4f7` and the Mac is now the one surface
                // out of step.
                //
                // "No runners" is deliberately not coloured. An app nobody has
                // added a runner to yet is empty, not broken.
                val down = runtimeIsDown(connections)
                val tint =
                    if (down) MaterialTheme.colorScheme.error
                    else MaterialTheme.colorScheme.onSurfaceVariant
                Icon(
                    Icons.Outlined.Dns,
                    null,
                    Modifier.size(16.dp),
                    tint = tint,
                )
                Spacer(Modifier.width(8.dp))
                Text(
                    liveSummary(connections),
                    style = MaterialTheme.typography.labelSmall,
                    color = tint,
                )
                Spacer(Modifier.weight(1f))
                TextButton(onClick = { addingRunner = true }) { Text("Add a runner") }
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
    if (addingRunner) {
        RunnerEditorSheet(
            existing = null,
            onSave = { model.addHost(it) },
            onRemove = null,
            onDismiss = { addingRunner = false },
        )
    }
    editingRunner?.let { host ->
        RunnerEditorSheet(
            existing = host,
            onSave = { model.hosts.update(it) },
            onRemove = { model.removeHost(it) },
            onDismiss = { editingRunner = null },
        )
    }
}

/**
 * Whether every runner this app knows about has an unreadable tmux.
 *
 * The same condition [liveSummary] turns into "tmux unavailable", asked
 * separately so the row can colour itself without parsing its own sentence.
 */
private fun runtimeIsDown(connections: List<Connection>): Boolean =
    connections.isNotEmpty() && connections.none { it.fleet.value.runtimeHealthy }

private fun liveSummary(connections: List<Connection>): String {
    val healthy = connections.count { it.fleet.value.runtimeHealthy }
    val live = connections.sumOf { it.fleet.value.livePanes }
    if (connections.isEmpty()) return "No runners"
    if (healthy == 0) return "tmux unavailable"
    val runners = if (connections.size == 1) "1 runner" else "${connections.size} runners"
    return "$live live · $runners"
}

/**
 * What one runner is doing, when that is not simply "answering".
 *
 * A runner that stops answering keeps its rows, dimmed, rather than dropping
 * them — so this row is what explains why they are stale. Every failure has
 * exactly one useful next move and they are not the same move, which is why
 * this switches on [Connection.Failure] rather than offering "try again" for
 * everything.
 */
@Composable
private fun RunnerStatusRow(
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
            // are this runner's last good answer and are still worth reading.
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
                    "This runner presented a key Far Cooler has never seen:",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    current.fingerprint,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                )
                Text(
                    "Check it on the host: " +
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
                val kind = current.kind
                // Red for the one failure that is genuinely alarming, and the
                // app's ordinary text for the rest. Every kind was red,
                // including DAEMON_MISSING — which is not a failure at all but a
                // runner that has never had `host install` run on it — and
                // KEY_NOT_TRUSTED and STOPPED, both of which this file's own
                // comments call "not a fault". Red on a step somebody simply has
                // not taken yet shouts about the wrong thing, and a colour spent
                // on everything is a colour that says nothing about the one case
                // that warrants it: a host key that changed underneath us.
                //
                // The same rule iOS's full-screen failure already follows, and
                // the same call the Mac makes for `notInstalled`, which it
                // paints `.secondary` in both `HostDot` and `troubleColor`. The
                // headline names what happened either way; it does not need the
                // colour to do it.
                Text(
                    failureHeadline(kind, connection),
                    style = MaterialTheme.typography.bodySmall,
                    color =
                        if (kind == Connection.Failure.HOST_KEY_CHANGED)
                            MaterialTheme.colorScheme.error
                        else MaterialTheme.colorScheme.onSurface,
                )
                Text(
                    failureDetail(kind, connection, current.message),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                // Only where the app has no diagnosis of its own, which is the
                // same scoping the Mac and the phone use: a transcript under a
                // sentence that already names the cause and the fix is noise.
                //
                // Nothing is discarded. For a runner nobody can reach, this
                // text is the only diagnosis that exists and somebody debugging
                // one needs it. It just goes where output goes rather than
                // where prose does, so the app stops appearing to have said it.
                if (kind == Connection.Failure.OTHER && current.message.isNotEmpty()) {
                    DetailBox(current.message, modifier = Modifier.padding(top = 6.dp))
                }
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
    Connection.Failure.HOST_KEY_CHANGED -> "This host’s key changed"
    Connection.Failure.UNREACHABLE -> "Can’t reach ${connection.host.address}"
    Connection.Failure.DAEMON_MISSING -> "Far Cooler isn’t installed"
    Connection.Failure.NO_IDENTITY -> "This device has no key"
    Connection.Failure.KEY_NOT_TRUSTED -> "Key not trusted"
    Connection.Failure.STOPPED -> "Stopped waiting"
    Connection.Failure.OTHER -> "Can’t connect"
}

/**
 * Ours wherever we know what happened, the core's own text only where we do
 * not. The raw string crossing up from Rust is written for whoever is reading a
 * log — lowercase, ending in things like "(os error 61)" — and putting that in
 * front of someone who just wants their runner back is asking them to
 * translate.
 *
 * The `else` arm is not raw text. Four of its five cases are sentences somebody
 * wrote — the changed host key's carries the two fingerprints being compared
 * and comes from `crates/client/src/ssh.rs`, the other three from `Connection`
 * and `Identity` — and they are the core's words only in the sense that the
 * core is where they are stored.
 */
private fun failureDetail(
    kind: Connection.Failure,
    connection: Connection,
    message: String,
): String = when (kind) {
    Connection.Failure.KEY_REJECTED ->
        "${connection.host.user}@${connection.host.address} hasn’t been given this device’s key."

    Connection.Failure.UNREACHABLE ->
        "Nothing answered on port ${connection.host.port}. The runner may be asleep, " +
            "or the address may be wrong."

    Connection.Failure.DAEMON_MISSING ->
        "SSH connected, but the Far Cooler daemon didn’t answer. Install it there."

    // The undiagnosed arm, and the only one where `message` is whatever came
    // back rather than something written to be read. Those words go in a
    // `DetailBox` above instead of standing here as the app's own account of
    // the runner.
    //
    // No cause named, deliberately: from this side the cause is unknowable, and
    // a guess sends somebody to loosen an sshd setting that was never the
    // problem. See `Enrollment.note(about:outcome:)` in the Mac app. Nor any
    // retry promised — whether one is under way is `retryOrGiveUp`'s
    // business, and the button below is the only offer this row makes.
    Connection.Failure.OTHER -> "The attempt to reach it didn’t finish."

    else -> message
}

@Composable
private fun WorkspaceHeader(
    entry: FleetEntry,
    showRunner: Boolean,
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
                    if (showRunner) append(" · ${entry.host.displayLabel}")
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
                Icon(Icons.Filled.MoreVert, contentDescription = "Workspace actions")
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
        ProcessDot(kind)
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
        //
        // One size. This stepped 16dp to 20dp on `wantsAttention`, so the
        // trailing column changed width every time an agent finished a turn or
        // asked a question — and it changed for ONE row, which pulls that row's
        // glyph out of line with the twenty above and below it. A column that
        // moves as its rows change state is a defect on any platform, and the
        // Mac's `StatusGlyph` spends its whole doc comment on the same point.
        // Emphasis is carried by fill and by colour instead, in channels that
        // cost no layout: PanTool and CheckCircle are filled where PauseCircle
        // and Pending are outlined.
        //
        // [attentionColor] takes the TERMINAL, not its activity, so a turn that
        // died is red rather than wearing the green of one that succeeded.
        if (terminal.agent.isAgent && terminal.agent != AgentActivity.UNKNOWN) {
            Icon(
                activityIcon(terminal),
                contentDescription = terminal.activityLabel,
                tint = attentionColor(terminal),
                modifier = Modifier.size(18.dp),
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

/**
 * The same vocabulary the Apple apps use, in this platform's icon set.
 *
 * NONE and UNKNOWN are unreachable from every call site — both guard on
 * `isAgent && != UNKNOWN` before drawing anything — and are kept because the
 * `when` is exhaustive over the enum, not because either has ever been on
 * screen. Recorded rather than deleted so the next reader does not go looking
 * for the terminal glyph.
 *
 * Filled where the state wants a person, outlined where it does not, which is
 * the non-colour half of the vocabulary the Mac calls hollow-vs-filled. It
 * happens to be right here already; the process dot beside it is where the
 * phones had lost it.
 */
fun activityIcon(activity: AgentActivity): ImageVector = when (activity) {
    AgentActivity.NONE -> Icons.Outlined.Terminal
    AgentActivity.IDLE -> Icons.Outlined.PauseCircle
    AgentActivity.WORKING -> Icons.Outlined.Pending
    AgentActivity.BLOCKED -> Icons.Filled.PanTool
    AgentActivity.DONE -> Icons.Filled.CheckCircle
    AgentActivity.UNKNOWN -> Icons.AutoMirrored.Outlined.HelpOutline
}

/**
 * The same mark, for a terminal whose finished turn may have DIED.
 *
 * `Cancel` is Material's filled circle with a cross in it — the same mark iOS
 * draws as `xmark.circle.fill`, and the same filled shape as the CheckCircle it
 * replaces, so a failed turn and a finished one differ in what is inside the
 * circle as well as in its colour.
 */
fun activityIcon(terminal: Terminal): ImageVector =
    if (terminal.turnDidFail) Icons.Filled.Cancel else activityIcon(terminal.agent)
