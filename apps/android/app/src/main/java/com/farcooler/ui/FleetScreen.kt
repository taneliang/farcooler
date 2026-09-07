package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.outlined.AccountTree
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material.icons.outlined.Settings
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
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.data.Reach
import com.farcooler.data.Runner
import com.farcooler.model.AgentActivity
import com.farcooler.model.GlanceMarkSize
import com.farcooler.model.WorkspaceOrder
import com.farcooler.model.StateKind
import com.farcooler.model.Terminal
import com.farcooler.model.Workspace
import com.farcooler.net.Connection
import com.farcooler.net.FleetEntry
import com.farcooler.net.TerminalRef
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Every workspace on every runner, in one scroll area.
 *
 * Shown two places — as a whole screen pushed from the front door's Workspaces
 * row, and inside the drawer over anything — so a task started from either one
 * works the same way and neither loses a capability the other has.
 *
 * Not a front door, and this is the list that made that clear: it holds every
 * worktree whether or not it wants anything, in creation order, with the two
 * buttons that start new work. That is the right answer to "where is my stuff"
 * and the wrong answer to "what needs me" — see `NeedsYouScreen`, which asks
 * the second question and links here for the first.
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

/**
 * The fleet as a whole screen.
 *
 * No longer where the app lands when nothing is running — the front door is,
 * always — so this is now a destination somebody chose: pushed from the front
 * door's Workspaces row, which is also the only place in the app that says how
 * many there are.
 *
 * [onBack] is what says so. It carries the back arrow, and the hamburger is
 * kept only for the case where this screen is the ground with nothing under it,
 * which the route stack no longer produces but `Ground`'s fallback still can.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FleetScreen(
    model: AppModel,
    onSelect: (TerminalRef) -> Unit,
    onOpenDrawer: () -> Unit,
    onBack: (() -> Unit)? = null,
) {
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Workspaces") },
                navigationIcon = {
                    if (onBack != null) {
                        IconButton(onClick = onBack) {
                            Icon(
                                Icons.AutoMirrored.Filled.ArrowBack,
                                contentDescription = "Back",
                            )
                        }
                    } else {
                        IconButton(onClick = onOpenDrawer) {
                            Icon(Icons.Outlined.Menu, contentDescription = "Show the fleet")
                        }
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
    // Saveable: the only state on this screen that is a decision rather than a
    // transient. The sheets below deliberately are not — a process death is not
    // a reason to reopen a half-filled "new workspace" form on somebody's
    // behalf. `rememberLazyListState` already saves the scroll position itself.
    var showHidden by rememberSaveable { mutableStateOf(false) }
    var editingRunner by remember { mutableStateOf<com.farcooler.data.Runner?>(null) }
    var addingRunner by remember { mutableStateOf(false) }
    var newTerminalIn by remember { mutableStateOf<Pair<Connection, Workspace>?>(null) }
    // The two phase-8 doors off a workspace row. Both hold a `Connection` as
    // well as the workspace, because a workspace id means nothing without the
    // runner that minted it — the same rule `ChangesStores` is keyed by.
    var stackFor by remember { mutableStateOf<FleetEntry?>(null) }
    var removing by remember { mutableStateOf<FleetEntry?>(null) }

    // Naming the runner only earns its place once there is more than one.
    // With a single runner connected its name is on every row and says
    // nothing about which row is which.
    val namesRunners = connections.size > 1

    val visible = entries.filter { showHidden || !it.workspace.isHidden }
    val hiddenCount = entries.count { it.workspace.isHidden }

    // The list's own state, because a drag has to ask where things ARE. Nothing
    // else on this screen needed it — `rememberLazyListState` saves the scroll
    // position either way.
    val listState = rememberLazyListState()
    // The card being held, by list key, and where letting go would put it.
    // Deliberately not `rememberSaveable`: a drag interrupted by a process death
    // is a drag that did not happen.
    var lifted by remember { mutableStateOf<String?>(null) }
    var landing by remember { mutableStateOf<WorkspaceOrder.Landing?>(null) }

    fun keyOf(entry: FleetEntry) = "${entry.host.id}/${entry.workspace.id}"

    // Only cards on the SAME runner take part in a drag. Each runner keeps its
    // own order in its own database, so a card cannot move into another
    // runner's stretch of this list — and a finger that wanders into one must
    // not be read as asking for that. Clamping to this runner's own cards is
    // what turns such a wander into "the end of my own stretch", which is
    // almost always what was meant.
    fun spans(hostId: String): List<WorkspaceOrder.Card> {
        val mine = visible.filter { it.host.id == hostId }.map(::keyOf).toSet()
        val laid = listState.layoutInfo.visibleItemsInfo.map {
            WorkspaceOrder.Laid(it.key.toString(), it.offset, it.size)
        }
        return WorkspaceOrder.cards(laid, mine)
    }

    // Let go: work out the runner's new order and send it, or send nothing.
    fun commitDrag() {
        val dragged = lifted
        val landed = landing
        lifted = null
        landing = null
        if (dragged == null || landed == null) return
        val entry = visible.firstOrNull { keyOf(it) == dragged } ?: return
        val group = visible.filter { it.host.id == entry.host.id }
        val order = group.map(::keyOf)
        val next = WorkspaceOrder.moved(order, dragged, landed.target, landed.edge)
        // A drop that changes nothing costs no round trip. It is not free: a
        // reorder makes every other client of that runner re-read the fleet.
        if (next == order) return
        val ids = next.mapNotNull { key -> group.firstOrNull { keyOf(it) == key }?.workspace?.id }
        scope.launch { entry.connection.reorderWorkspaces(ids) }
    }

    LazyColumn(state = listState, modifier = modifier, contentPadding = contentPadding) {
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
                    drag = HeaderDrag(
                        key = keyOf(entry),
                        // Offered only where the runner keeps an order. A daemon
                        // that predates `workspace.reorder` sends no `ordinal`,
                        // and a drag against one would rearrange the screen and
                        // put it all back on the next refresh with nothing
                        // failing anywhere.
                        enabled = entry.workspace.ordinal != null,
                        lifted = lifted == keyOf(entry),
                        edge = landing?.takeIf { it.target == keyOf(entry) }?.edge,
                        onStart = {
                            lifted = keyOf(entry)
                            landing = null
                        },
                        // The finger's position arrives relative to this card;
                        // where the cards are is in the list's coordinates. This
                        // card's own laid-out offset is what joins the two.
                        onMove = { y ->
                            val me = listState.layoutInfo.visibleItemsInfo
                                .firstOrNull { it.key == keyOf(entry) }
                            if (me != null) {
                                landing = WorkspaceOrder.landing(
                                    spans(entry.host.id), me.offset + y.toInt())
                            }
                        },
                        onEnd = { commitDrag() },
                        onCancel = {
                            lifted = null
                            landing = null
                        },
                    ),
                    onHide = { hidden ->
                        scope.launch { entry.connection.setHidden(entry.workspace, hidden) }
                    },
                    onNewTerminal = { newTerminalIn = entry.connection to entry.workspace },
                    onStack = { stackFor = entry },
                    onRemove = { removing = entry },
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
                // exactly the color of "3 live · 2 runners" — the one sentence
                // on this screen that means every pane on every runner is
                // unreadable, drawn as though it were a healthy count. Both the
                // Mac and iOS give the same three words a colored mark; this
                // one already has a mark of its own, so the mark takes the
                // color rather than a second dot being added beside it.
                //
                // Red rather than the Mac's amber. Amber means an agent is
                // waiting on you, and nobody is waiting here: the runtime every
                // pane lives inside is not answering, which is a failure. iOS
                // settled that in `7e4a4f7` and the Mac is now the one surface
                // out of step.
                //
                // "No runners" is deliberately not colored. An app nobody has
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
    // The menu only offers this when the runner named the repository, so the
    // inner `let` is a compiler obligation rather than a real case — and drawing
    // nothing is the right answer if it ever becomes one. Deliberately not
    // clearing `stackFor` here instead: writing state during composition is how
    // a screen recomposes itself in a loop, and Cancel already clears it.
    stackFor?.let { entry ->
        entry.workspace.repository?.let { repository ->
            StackSheet(
                connection = entry.connection,
                repository = repository,
                branch = entry.workspace.branch,
                onDismiss = { stackFor = null },
            )
        }
    }
    removing?.let { entry ->
        RemoveWorktreeCeremony(
            connection = entry.connection,
            workspace = entry.workspace,
            onFinished = { removing = null },
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
 * separately so the row can color itself without parsing its own sentence.
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
 * A runner that stops answering keeps its rows rather than dropping them, so
 * this row is what explains why they are stale. It said "dimmed" here for as
 * long as it has existed and nothing has ever dimmed them — see
 * [Connection.Phase.Reconnecting] for the drift and what closing it would cost.
 *
 * Every failure has exactly one useful next move and they are not the same
 * move, which is why this switches on [Connection.Failure] rather than offering
 * "try again" for everything.
 *
 * **Shared with the front door, which is why this is not private.** A runner
 * that failed and a runner that is reconnecting are two states iOS's
 * single-connection inbox never had to put in a LIST, and they are exactly the
 * states that stop "nothing needs you" from being a true sentence. The front
 * door draws THESE rows rather than a second, shorter version of them, because
 * two screens offering different next moves about the same runner is the drift
 * this codebase keeps finding.
 */
@Composable
internal fun RunnerStatusRow(
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
                // not taken yet shouts about the wrong thing, and a color spent
                // on everything is a color that says nothing about the one case
                // that warrants it: a host key that changed underneath us.
                //
                // The same rule iOS's full-screen failure already follows, and
                // the same call the Mac makes for `notInstalled`, which it
                // paints `.secondary` in both `HostDot` and `troubleColor`. The
                // headline names what happened either way; it does not need the
                // color to do it.
                Text(
                    failureHeadline(kind, connection.host),
                    style = MaterialTheme.typography.bodySmall,
                    color =
                        if (kind == Connection.Failure.HOST_KEY_CHANGED)
                            MaterialTheme.colorScheme.error
                        else MaterialTheme.colorScheme.onSurface,
                )
                Text(
                    failureDetail(kind, connection.host, current.message),
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

                        // No "Try again": the dial would use the key that is
                        // missing, so the button could only fail, every time,
                        // forever. The sentence above names the one thing that
                        // works, which is this device asking to be added again.
                        Connection.Failure.NO_NODE_KEY -> Unit

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

/**
 * The headline over a failed runner.
 *
 * Takes the [Runner] rather than the [Connection] so a JVM unit test can call
 * it: a `Connection` holds a `ClientCore` and a coroutine scope and cannot be
 * built off a device, and copy nothing reads back is copy that drifts. The two
 * things a sentence here needs — what to call this runner, and how it is
 * reached — are both on the runner. `TunnelCopyTest` is what reads it.
 */
internal fun failureHeadline(kind: Connection.Failure, runner: Runner): String = when (kind) {
    Connection.Failure.KEY_REJECTED -> "Not authorized yet"
    Connection.Failure.HOST_KEY_CHANGED -> "This host’s key changed"
    Connection.Failure.UNREACHABLE -> "Can’t reach ${runner.named}"
    Connection.Failure.DAEMON_MISSING -> "Far Cooler isn’t installed"
    Connection.Failure.NO_IDENTITY -> "This device has no key"
    Connection.Failure.NO_NODE_KEY -> "This device has no tunnel key"
    Connection.Failure.KEY_NOT_TRUSTED -> "Key not trusted"
    Connection.Failure.STOPPED -> "Stopped waiting"

    // Named the way [Connection.Failure.UNREACHABLE] names it, because it is
    // the same fact about the same runner: nothing answered. `Runner.named` is
    // what makes that a label rather than the empty address a tunneled runner
    // has.
    Connection.Failure.TUNNEL_NO_ANSWER -> "Can’t reach ${runner.named}"

    // Not the runner's name: what could not be reached is the rendezvous, and
    // blaming the runner would send somebody to go and wake a machine that was
    // awake the whole time.
    Connection.Failure.TUNNEL_RENDEZVOUS -> "Can’t reach the tunnel"
    Connection.Failure.TUNNEL_NOT_IN_THIS_BUILD -> "No tunnel in this build"
    Connection.Failure.TUNNEL_UNSPECIFIED -> "The tunnel didn’t open"

    Connection.Failure.OTHER -> "Can’t connect"
}

/**
 * Ours wherever we know what happened, the core's own text only where we do
 * not. The raw string crossing up from Rust is written for whoever is reading a
 * log — lowercase, ending in things like "(os error 61)" — and putting that in
 * front of someone who just wants their runner back is asking them to
 * translate.
 *
 * There is deliberately no `else` arm. Five kinds pass [message] through and
 * every one of them is a sentence somebody wrote — the changed host key's
 * carries the two fingerprints being compared and comes from
 * `crates/client/src/ssh.rs`, the other four from `Connection` and `Identity` —
 * and they are the core's words only in the sense that the core is where they
 * are stored. An `else` here is what would let a kind added later pass the
 * core's own text through instead, silently, which is exactly how
 * `cannot open the tunnel: no_answer` used to reach a screen. Naming all
 * thirteen makes a fourteenth a compile error.
 *
 * Takes the [Runner] rather than the [Connection] for [failureHeadline]'s
 * reason: so a test can read this copy back.
 */
internal fun failureDetail(
    kind: Connection.Failure,
    runner: Runner,
    message: String,
): String = when (kind) {
    Connection.Failure.KEY_REJECTED ->
        runner.reach.detail(runner.user) + " hasn’t been given this device’s key."

    // No port to name and no address to have got wrong for a tunneled runner:
    // it is reached by token, so the two things a person could check are whether
    // the runner is awake and whether it is on the tunnel.
    Connection.Failure.UNREACHABLE -> when (val reach = runner.reach) {
        is Reach.Direct ->
            "Nothing answered on port ${reach.port}. The runner may be asleep, " +
                "or the address may be wrong."
        is Reach.Tailcat ->
            "The tunnel didn’t reach it. The runner may be asleep, or off the tunnel."
    }

    Connection.Failure.DAEMON_MISSING ->
        "SSH connected, but the Far Cooler daemon didn’t answer. Install it there."

    // The app's own sentences for the tunnel's four stable words. Never
    // [message]: that is `cannot open the tunnel: <word>`, and the word is the
    // thing this table exists to keep off a screen.
    //
    // `crates/cli/src/runner_pipe.rs`'s `sentence` says the same four things to
    // whoever is reading a terminal, and `RunnerTrouble` in
    // `apps/shared/AgentKit` says them on the Apple apps. Reword one and the
    // other two are where to look.
    //
    // Both causes named for the first, because from here they are
    // indistinguishable — a revoked device is ignored silently and times out
    // exactly as a sleeping runner does — and naming only one would send half
    // the people who read this to the wrong place.
    Connection.Failure.TUNNEL_NO_ANSWER ->
        "It didn’t answer. The runner may be asleep, or this device’s access " +
            "to it may have been revoked."

    Connection.Failure.TUNNEL_RENDEZVOUS ->
        "The service that introduces this device to the runner didn’t answer. " +
            "Check this device’s own network."

    Connection.Failure.TUNNEL_NOT_IN_THIS_BUILD ->
        "This build of Far Cooler has no tunnel it can dial."

    // No cause named, for OTHER's reason: `io` is deliberately generic upstream,
    // so a guess here would send somebody to fix something that was never the
    // problem.
    Connection.Failure.TUNNEL_UNSPECIFIED -> "The tunnel couldn’t be opened."

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

    Connection.Failure.HOST_KEY_CHANGED,
    Connection.Failure.NO_IDENTITY,
    Connection.Failure.NO_NODE_KEY,
    Connection.Failure.KEY_NOT_TRUSTED,
    Connection.Failure.STOPPED,
    -> message
}

/**
 * Everything a workspace header needs to be draggable, in one argument.
 *
 * One parameter rather than seven, because this header already carries five and
 * a call site with a dozen positional lambdas is where the wrong one gets passed
 * without the compiler noticing — every one of these is `() -> Unit`.
 */
private class HeaderDrag(
    val key: String,
    val enabled: Boolean,
    val lifted: Boolean,
    /** The edge to draw an insertion line on, or null if this is not the target. */
    val edge: WorkspaceOrder.Edge?,
    val onStart: () -> Unit,
    /** The finger, in this card's own coordinates. */
    val onMove: (Float) -> Unit,
    val onEnd: () -> Unit,
    val onCancel: () -> Unit,
)

@Composable
private fun WorkspaceHeader(
    entry: FleetEntry,
    showRunner: Boolean,
    drag: HeaderDrag,
    onHide: (Boolean) -> Unit,
    onNewTerminal: () -> Unit,
    onStack: () -> Unit,
    onRemove: () -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    Box(
        Modifier
            .fillMaxWidth()
            // After a long press, not on touch. A short drag on this list is a
            // scroll, and it has to stay one: taking the gesture immediately
            // would make a fleet of twenty worktrees unscrollable from the one
            // place a thumb naturally lands.
            .then(
                if (!drag.enabled) Modifier
                else Modifier.pointerInput(drag.key) {
                    detectDragGesturesAfterLongPress(
                        onDragStart = { drag.onStart() },
                        onDrag = { change, _ ->
                            change.consume()
                            drag.onMove(change.position.y)
                        },
                        onDragEnd = { drag.onEnd() },
                        onDragCancel = { drag.onCancel() },
                    )
                }
            )
            // Held. The card being dragged has to be visible as the one that
            // moved, or a list where two rows look alike gives no feedback at
            // all about what is in the air.
            .background(
                if (drag.lifted) MaterialTheme.colorScheme.surfaceVariant
                else androidx.compose.ui.graphics.Color.Transparent
            )
    ) {
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
                // Only offered when the runner said which repository this
                // worktree belongs to and which branch it is on. An older
                // daemon's fleet carried neither, and a menu item that cannot
                // work is worse than one that is not there.
                if (entry.workspace.repository != null && entry.workspace.branch.isNotBlank()) {
                    DropdownMenuItem(
                        text = { Text("Stack & pull request") },
                        onClick = {
                            menu = false
                            onStack()
                        },
                    )
                }
                DropdownMenuItem(
                    text = { Text(if (entry.workspace.isHidden) "Unhide" else "Hide") },
                    onClick = {
                        menu = false
                        onHide(!entry.workspace.isHidden)
                    },
                )
                // **Never for the repository's own checkout.** Removing it would
                // offer to delete the directory the repository itself lives in,
                // and the daemon refuses it twice — the stored flag, then a path
                // comparison that deliberately does not trust the flag. Keeping
                // the item off the menu is not what makes that safe; it is what
                // keeps nobody walking through a destructive ceremony that could
                // never have succeeded. See `07e75e8`, which is that story on
                // iOS, and `RemoveWorktreeCeremony`.
                if (!entry.workspace.isMainCheckout) {
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                    DropdownMenuItem(
                        text = {
                            Text(
                                "Remove worktree…",
                                color = MaterialTheme.colorScheme.error,
                            )
                        },
                        onClick = {
                            menu = false
                            onRemove()
                        },
                    )
                }
            }
        }
    }

        // Where letting go would put the card, drawn on the edge it would
        // insert at. A line rather than a highlighted card: the question is
        // which GAP the card goes into, and a lit card says "on top of this
        // one", which is a thing this gesture cannot do.
        //
        // An overlay rather than a sibling above or below the row, so appearing
        // does not change the card's height — a list that resizes under a finger
        // moves the very targets being aimed at.
        if (drag.edge != null) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(2.dp)
                    .align(
                        if (drag.edge == WorkspaceOrder.Edge.ABOVE) Alignment.TopCenter
                        else Alignment.BottomCenter
                    )
                    .background(MaterialTheme.colorScheme.primary)
            )
        }
    }
}

/**
 * One pane, in four bands.
 *
 * The bands, in the Mac's order and iOS's: what it is and how long it has been
 * that; where the agent IS; what it said; what it spawned. The second band used
 * to be `terminal.state.lowercase()` — the raw wire word, which restated the dot
 * immediately to its left in the most valuable line of the row. Everything that
 * replaced it was already arriving on every poll and being dropped on the way
 * into [Terminal].
 *
 * **A `Row`, not a `ListItem`.** Material's `ListItem` has exactly three text
 * slots and a specified minimum height per one-, two- and three-line variant;
 * this row is four bands whose count varies from one to eight lines as an agent
 * works, and the only way to express that through `ListItem` is to put the whole
 * stack in `supportingContent`, which then gets the three-line variant's padding
 * around content twice that tall. It also already was not one: this row has a
 * leading dot and a trailing overflow menu that `ListItem` would re-pad. The
 * shape below is the one the content has.
 *
 * **Aligned to the top, not the centre.** A one-line row and an eight-line row
 * are in the same list, and a dot centred in the second would sit four lines
 * below the name it belongs to.
 *
 * **Shared with the front door, which is why this is not private.** The front
 * door draws agents with THIS row and no other. A second way to draw an agent is
 * a second chance for two screens to say different things about one pane — iOS
 * makes the same call in `NeedsYou.swift` for the same reason, and it is why the
 * front door uses `ListItem` for its other two rows and not for this one:
 * `ListItem`'s three text slots cannot hold four bands, as the note above
 * already argues.
 *
 * That paragraph arrived as a SECOND KDoc block stacked under this one when the
 * front door started sharing the row, and Kotlin binds only the last of those —
 * so everything above it, the whole argument for four bands and for a `Row`
 * rather than a `ListItem`, documented nothing. Both halves were right; only the
 * arrangement was wrong. `Connection.setHidden` had the same thing happen to it
 * and is fixed the same way.
 */
@Composable
internal fun TerminalRow(
    terminal: Terminal,
    ordinal: Int?,
    onClick: () -> Unit,
    onAction: (Connection.Action) -> Unit,
) {
    val kind = StateKind.parse(terminal.state)
    var menu by remember { mutableStateOf(false) }
    // Not ticking, deliberately, where [ElapsedStatus] below does tick. The one
    // question asked of this clock is whether the runner's last answer is over
    // an hour old, and a value taken when this row was last composed is exact
    // enough for an hour: a fleet poll recomposes it long before the threshold
    // could be crossed unobserved. A second one-second timer per row, to move a
    // dash that changes once an hour, is a timer for nothing.
    val now by rememberNow(ticking = false)

    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.Top,
    ) {
        // Centred on the FIRST LINE, by giving the dot a box exactly that line
        // tall — not by a hardcoded top padding, which is what iOS had to
        // correct: a measured offset is right at one text size and wrong at
        // every other, and the first line's height moves with the type scale
        // while a number in a source file does not.
        Box(Modifier.height(firstLineHeight()), contentAlignment = Alignment.Center) {
            ProcessDot(kind)
        }
        Spacer(Modifier.width(10.dp))

        // A full step between the two groups, a tight one inside each. What the
        // pane IS and where its agent got to are one thought in two lines; what
        // the agent SAID is a different thought, and the gap is what says so.
        //
        // Smaller than iOS's 4 and 8 for the same rhythm, because Material's
        // type scale carries its leading in `lineHeight`: every `Text` here is
        // already taller than its glyphs, where a SwiftUI `Text` is not.
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(BAND_STEP)) {
            Column(verticalArrangement = Arrangement.spacedBy(BAND_TIGHT)) {
                // Band 1: what it is, and how long it has been that.
                Row(verticalAlignment = Alignment.CenterVertically) {
                    // The name, the ordinal and the elapsed status pack to the
                    // left inside their own weighted row, so the glyph after it
                    // sits at the row's right edge rather than wherever the name
                    // happened to end — and so a long conversation title is the
                    // thing that truncates, rather than pushing the state off
                    // the screen.
                    Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
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
                                // The faintest thing on the row, deliberately.
                                // It is a disambiguator between two identical
                                // `claude` panes and nothing more, and it must
                                // not read as loud as the agent's own words
                                // three lines below it — which is the mistake
                                // iOS made in the other direction, drawing those
                                // words in the ordinal's tier. Material has no
                                // tertiary text role, so the step down is an
                                // alpha on the same color rather than a second
                                // color nobody defined.
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                                    .copy(alpha = 0.6f),
                            )
                        }
                        ElapsedStatus(terminal)
                    }

                    // The reason to have opened the app: one mark, in the one
                    // vocabulary every surface in the product now draws.
                    //
                    // **The ring is your side, the core is the agent's** — for
                    // the states §03 has a mark for. A finished turn and one
                    // that died are not among them: they keep the green and the
                    // red they already had, because the nearest mark for either
                    // is the quiet hairline and "nothing is wanted from you" is
                    // the opposite of what a failed build means. [AgentMarkView]
                    // holds that fork, and holds it once for every surface. See
                    // `model/Glance.kt` and the Mac's `Status.glanceMark`.
                    //
                    // **What this replaced, and why the loss is the point.**
                    // Six Material icons tinted orange-or-green-or-red said
                    // six things where the row says three: `rowStatus` already
                    // prints "Needs you 2m", "Working 12m" and "Failed", so the
                    // glyph was restating a string beside it in a channel — hue
                    // — that a colorblind reader, a greyscale screenshot and a
                    // phone in sunlight all lose. §03's mark keeps the
                    // distinction that survives all three: stroke weight for
                    // whether you are wanted, a filled centre for whether it is
                    // producing, a dash for whether we have heard from it. The
                    // two hues that survive are the two that are NOT restating a
                    // tier — green and red are an outcome, and a chip elsewhere
                    // in the app carries them with no words beside it at all.
                    //
                    // One size still, and now a smaller one — 10dp, §03's row
                    // diameter. The column cannot change width as a row changes
                    // state, which is the defect the icon's own comment was
                    // written to prevent and which the mark prevents by
                    // construction: every state is the same circle.
                    if (terminal.agent.isAgent && terminal.agent != AgentActivity.UNKNOWN) {
                        Spacer(Modifier.width(8.dp))
                        AgentMarkView(
                            terminal,
                            now,
                            GlanceMarkSize.ROW,
                            // Decorative, because the words are right there.
                            // `rowStatus` is never null for an agent pane — it
                            // falls back to `activityLabel` alone — so whatever
                            // this draws is already in the row's text, and a
                            // mark that announced it too would make TalkBack say
                            // "Needs you" twice per row.
                            decorative = true,
                        )
                    }
                }

                // Band 2: where the agent IS — the question it is blocked on,
                // its position in its own task list, or what it is doing. One
                // line, composed on the host so a Mac, a phone and a watch
                // cannot disagree about which of those three to show.
                //
                // This is what replaced the raw state word.
                if (terminal.signalLine.isNotEmpty()) {
                    Text(
                        terminal.signalLine,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }

            // Bands 3 and 4: what the agent said it did, and what it spawned and
            // has not finished with.
            //
            // The part of the row that answers "what did it do", which is most
            // of what reviewing an agent's work is — so they are drawn in the
            // ordinary supporting color, not the faint one the ordinal gets.
            //
            // Guarded as a group rather than left loose: an empty column would
            // still take the step of spacing above it and leave a gap under
            // every one-agent row.
            if (terminal.recentSteps.isNotEmpty() || terminal.runningSubagents.isNotEmpty()) {
                Column(verticalArrangement = Arrangement.spacedBy(BAND_TIGHT)) {
                    // Already redacted and cut to a row's width by the daemon,
                    // so this renders them and decides nothing about them.
                    for (step in terminal.recentSteps) {
                        Text(
                            step,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }

                    for (name in terminal.runningSubagents) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            // An icon where the Apple apps print U+2442 in front
                            // of the name. Android has no guarantee that the
                            // system font covers that character, and a tofu box
                            // in front of every subagent is worse than either
                            // the glyph or nothing — the one thing this row must
                            // never do is look broken while reporting healthy
                            // work.
                            Icon(
                                Icons.Outlined.AccountTree,
                                null,
                                Modifier.size(12.dp),
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Spacer(Modifier.width(4.dp))
                            Text(
                                name,
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                }
            }
        }

        // Top-aligned with everything else, so it stays beside the name as the
        // row grows rather than drifting to the middle of an eight-line one. Its
        // 48dp target is left alone: a menu you have to aim at on a moving train
        // is the wrong thing to shrink.
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

/** Between the two groups of bands, and between the lines inside one group. */
private val BAND_STEP = 6.dp
private val BAND_TIGHT = 2.dp

/**
 * How tall the row's first line is, so the process dot can be centred on it.
 *
 * Read from the type scale rather than written down, because the whole point is
 * that it moves when the reader's text size does. `bodyLarge` is what the pane's
 * name is set in; if that ever changes, this follows it.
 *
 * The fallback is for a theme that leaves `lineHeight` unspecified, which
 * Material's own type scale does not — `toDp()` on an unspecified `TextUnit`
 * throws, and a crash in a list row is not worth the four bytes of not checking.
 */
@Composable
private fun firstLineHeight(): Dp {
    val lineHeight = MaterialTheme.typography.bodyLarge.lineHeight
    if (!lineHeight.isSp) return 24.dp
    return with(LocalDensity.current) { lineHeight.toDp() }
}

/**
 * "Working 12m", "Needs you 2m" — the half of band one that moves.
 *
 * Its own composable, and that is the whole trick. The clock is read HERE, so
 * only this `Text` is invalidated when the second changes; the name beside it,
 * the three lines of transcript below it and the twenty other rows in the list
 * are not. iOS wraps its entire row in a `TimelineView` and re-evaluates the
 * whole subtree once a second because SwiftUI gives it nowhere smaller to put
 * the clock. Compose does, and this is it.
 *
 * A row with nothing to count runs no coroutine at all — see [Terminal.hasClock]
 * — so twenty idle panes cost twenty suspended-forever `produceState`s and no
 * wakeups.
 */
@Composable
private fun ElapsedStatus(terminal: Terminal) {
    val now by rememberNow(ticking = terminal.hasClock)
    val status = terminal.rowStatus(now) ?: return
    Text(
        status,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = 1,
        modifier = Modifier.padding(start = 6.dp),
    )
}

/**
 * The wall clock, advancing once a second while [ticking] and standing still
 * otherwise.
 *
 * `System.currentTimeMillis` rather than `SystemClock.elapsedRealtime`, and
 * deliberately, where the pairing ceremony's freshness window uses the other
 * one: the timestamps this is subtracted from were taken on the RUNNER, so the
 * only clock that can be compared with them is the one that claims to tell the
 * same time. A monotonic clock is right for measuring a window this device
 * opened and wrong for measuring against a moment another machine recorded.
 */
@Composable
private fun rememberNow(ticking: Boolean): State<Long> =
    produceState(System.currentTimeMillis(), ticking) {
        value = System.currentTimeMillis()
        while (ticking) {
            delay(1_000)
            value = System.currentTimeMillis()
        }
    }

