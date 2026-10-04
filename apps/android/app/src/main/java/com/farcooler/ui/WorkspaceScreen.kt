package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Tab
import androidx.compose.material3.TabRow
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.core.CoreException
import com.farcooler.model.AgentHarness
import com.farcooler.model.FirstRunCopy
import com.farcooler.model.PhoneEmptyStates
import com.farcooler.model.HarnessAvailability
import com.farcooler.model.OrchestratorExit
import com.farcooler.model.RunnerLink
import com.farcooler.model.WorktreeScope
import com.farcooler.net.Connection
import com.farcooler.net.TerminalRef
import com.farcooler.net.rethrowIfCancellation
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * One workspace: its name, and a tab row of Orchestrator, Board and Worktrees
 * under the top app bar (spec §6.2).
 *
 * - **Orchestrator** is the orchestrator's pane, full height, drawn by the same
 *   [TerminalPane] a worktree uses — its chat or its terminal. With none,
 *   Start Orchestrator (ruling 8); with a lost one, Restart and Replace.
 * - **Board** is the list form of the workspace's board ([BoardTab]).
 * - **Worktrees** is the workspace's worktrees ([WorktreeList]), with New
 *   Worktree at its head, which claims the new one for this workspace.
 *
 * The tab is in the route, and the screen is keyed on the workspace alone, so
 * a tab tap rebuilds nothing. The orchestrator's pane stays composed while
 * another tab is up — hidden, and holding no traffic — so coming back to it
 * doesn't reload the conversation.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WorkspaceScreen(
    model: AppModel,
    route: Route.Workspace,
    /** Whether this is what's being looked at, rather than under a pushed screen. */
    onScreen: Boolean,
    onBack: () -> Unit,
) {
    val connection = model.fleet.connection(route.hostId) ?: run {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Text("That runner is no longer connected.")
        }
        return
    }
    val fleet by connection.fleet.collectAsStateWithLifecycle()
    val repositories by connection.repositories.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val connections by model.fleet.active.collectAsStateWithLifecycle()
    // Not `collectAsStateWithLifecycle`, for `WorktreeScreen`'s reason: this
    // is what stops the pane's stream when the app leaves the foreground.
    val foreground by model.foreground.collectAsState()
    val scope = rememberCoroutineScope()

    val link by connection.link.collectAsStateWithLifecycle()
    val presence = WorkspacePresence.of(route.workspaceId, fleet, repositories, link == RunnerLink.ANSWERING)
    val workspace = (presence as? WorkspacePresence.Found)?.workspace ?: run {
        WorkspaceMissing(presence, connection.host.displayLabel, onBack)
        return
    }
    val repositoryName = repositories.firstOrNull { it.id == workspace.repository }
        ?.let { it.displayName.ifEmpty { it.short } }
    val name = if (workspace.isImplicit) repositoryName ?: "Main" else workspace.name.ifEmpty { "Workspace" }
    val subtitle = listOfNotNull(
        repositoryName.takeIf { !workspace.isImplicit },
        connection.host.displayLabel.takeIf { connections.size > 1 },
    ).joinToString(" · ")

    // The seat: the orchestrator the runner names for this workspace, found
    // by its terminal's role, or the workspace's own record of it.
    val seated = remember(fleet) {
        fleet.worktrees.asSequence().flatMap { it.terminals.asSequence() }
            .firstOrNull { it.isOrchestrator && it.workspace == workspace.id }?.id
            ?: fleet.workspaces?.firstOrNull { it.id == workspace.id }?.orchestrator
    }
    var startedAt by remember(workspace.id) { mutableStateOf<Long?>(null) }
    var refusal by remember(workspace.id) { mutableStateOf<String?>(null) }
    // Which agent this phone asked for, and when, kept past the pane's arrival
    // so a quick exit 127 can be called "not installed" (ov-205).
    var asked by remember(workspace.id) { mutableStateOf<Pair<OrchestratorHarness, Long>?>(null) }
    var endedAfterMs by remember(workspace.id) { mutableStateOf<Long?>(null) }
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(startedAt) {
        while (startedAt != null) {
            now = System.currentTimeMillis()
            delay(1_000)
        }
    }
    val mayControl = daemon?.grantedScope != "read"
    val seat = OrchestratorSeat.of(
        seated = seated,
        worktrees = fleet.worktrees,
        startedAt = startedAt,
        now = now,
        canStart = !workspace.isImplicit && mayControl && daemon?.can("workstreams") != false,
    )
    // Seen: the phone's own start is confirmed, and the clock can stop.
    LaunchedEffect(seat) { if (seat is OrchestratorSeat.Live) startedAt = null }
    // First seen ended: how long after the ask, which is what the 127 rule needs.
    LaunchedEffect(seat) {
        val ask = asked
        if (seat is OrchestratorSeat.Lost && ask != null && endedAfterMs == null) {
            endedAfterMs = System.currentTimeMillis() - ask.second
        }
    }
    val missing: AgentHarness? = (seat as? OrchestratorSeat.Lost)?.let { lost ->
        val ended = endedAfterMs ?: return@let null
        if (OrchestratorExit.classify(lost.terminal.exitCode, ended) != OrchestratorExit.NOT_INSTALLED) return@let null
        asked?.first?.agent ?: AgentHarness.entries.firstOrNull { it.wire == lost.terminal.preset }
    }

    fun start(harness: OrchestratorHarness, replace: Boolean) {
        refusal = null
        startedAt = System.currentTimeMillis()
        asked = harness to startedAt!!
        endedAfterMs = null
        scope.launch {
            try {
                connection.startOrchestrator(workspace.id, harness.wire, replace)
            } catch (e: Exception) {
                e.rethrowIfCancellation()
                startedAt = null
                val core = e as? CoreException
                refusal = orchestratorRefusal(core?.word, core?.what, name, replace)
            }
        }
    }

    // The orchestrator's pane is being read while its tab is up, this screen
    // is on top, and the app is in front: suppress its banners and mark its
    // finished turn seen, as a worktree's pane does.
    val live = seat as? OrchestratorSeat.Live
    val reading = live?.terminal?.id?.takeIf { route.tab == WorkspaceTab.ORCHESTRATOR && onScreen }
    DisposableEffect(reading) {
        if (reading != null) {
            model.claimReading(connection, reading)
            scope.launch { connection.markVisibleSeen() }
        }
        onDispose {
            if (reading != null) model.releaseReading(connection, reading)
        }
    }

    Scaffold(
        topBar = {
            Column {
                TopAppBar(
                    title = {
                        Column {
                            Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            if (subtitle.isNotEmpty()) {
                                Text(
                                    subtitle,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                        }
                    },
                    navigationIcon = {
                        IconButton(onClick = onBack) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                        }
                    },
                    actions = {
                        if (route.tab == WorkspaceTab.ORCHESTRATOR) {
                            OrchestratorActions(
                                connection = connection,
                                seat = seat,
                                mayControl = mayControl && !workspace.isImplicit,
                                availability = daemon?.availability ?: HarnessAvailability(null),
                                onReplace = { start(it, replace = true) },
                                onOpenWorktree = { ref -> model.open(ref) },
                            )
                        }
                    },
                )
                val (labels, selected) = WorkspaceTab.row(route.tab)
                TabRow(selectedTabIndex = selected) {
                    WorkspaceTab.entries.forEachIndexed { index, tab ->
                        Tab(
                            selected = index == selected,
                            onClick = { model.selectTab(route, tab) },
                            text = { Text(labels[index]) },
                            modifier = Modifier.testTag("workspace-tab-${tab.name.lowercase()}"),
                        )
                    }
                }
            }
        }
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            // Mounted whenever there is a pane to mount, whichever tab is up,
            // and live only on its own tab.
            if (live != null) {
                val showing = route.tab == WorkspaceTab.ORCHESTRATOR
                Box(Modifier.fillMaxSize().alpha(if (showing) 1f else 0f).mountedPane(showing)) {
                    TerminalPane(
                        model = model,
                        ref = TerminalRef(route.hostId, live.worktreeId, live.terminal.id),
                        connection = connection,
                        worktree = fleet.worktrees.firstOrNull { it.id == live.worktreeId },
                        showRunner = connections.size > 1,
                        live = showing && onScreen && foreground,
                        onPickImage = {},
                        showTopBar = false,
                        onOpenDrawer = {},
                    )
                }
            }
            when (route.tab) {
                WorkspaceTab.ORCHESTRATOR -> if (live == null) {
                    OrchestratorEmpty(
                        seat = seat,
                        actions = seatActions(seat, mayControl && !workspace.isImplicit),
                        refusal = refusal,
                        availability = daemon?.availability ?: HarnessAvailability(null),
                        runner = connection.host.displayLabel,
                        missing = missing,
                        onStart = { start(it, replace = false) },
                        onReplace = { start(it, replace = true) },
                        onRestart = { terminal -> scope.launch { connection.act(Connection.Action.RESTART, terminal) } },
                    )
                }
                WorkspaceTab.BOARD -> BoardTab(
                    connection = connection,
                    workspace = workspace,
                    onOpenTask = { model.navigate(Route.BoardTask(route.hostId, route.workspaceId, it)) },
                    onOpenHistory = { status ->
                        model.navigate(Route.BoardHistory(route.hostId, route.workspaceId, status.wire))
                    },
                    onJump = { model.openFromBoard(it) },
                    orchestratorRunning = seat is OrchestratorSeat.Live || seat is OrchestratorSeat.Starting,
                    onShowOrchestrator = { model.selectTab(route, WorkspaceTab.ORCHESTRATOR) },
                )
                WorkspaceTab.WORKTREES -> WorktreeList(
                    model = model,
                    scope = WorktreeScope.OfWorkspace(route.hostId, workspace),
                    onSelect = { model.open(it) },
                    modifier = Modifier.fillMaxSize(),
                )
            }
        }
    }
}

/**
 * A workspace route with no workspace behind it: a spinner while the runner
 * hasn't said, or — once it has — the plain fact, with the way back.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun WorkspaceMissing(presence: WorkspacePresence, runner: String, onBack: () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Workspace") },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        }
    ) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).padding(32.dp).testTag("workspace-missing"),
            verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            if (presence is WorkspacePresence.Gone) {
                Text("This workspace is gone", style = MaterialTheme.typography.titleMedium)
                Text(
                    "It isn’t on $runner anymore.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            } else {
                CircularProgressIndicator()
            }
        }
    }
}

/**
 * The Orchestrator tab's own controls, in the top bar: chat or terminal, and a
 * menu with Open Worktree (the pane among its worktree's others) and Replace.
 */
@Composable
private fun OrchestratorActions(
    connection: Connection,
    seat: OrchestratorSeat,
    mayControl: Boolean,
    availability: HarnessAvailability,
    onReplace: (OrchestratorHarness) -> Unit,
    onOpenWorktree: (TerminalRef) -> Unit,
) {
    val scope = rememberCoroutineScope()
    val live = seat as? OrchestratorSeat.Live ?: return
    val terminal = live.terminal
    if (terminal.canSwitchPaneMode) {
        IconButton(onClick = {
            scope.launch { connection.setPaneMode(terminal, if (terminal.isAgentPane) "terminal" else "agent") }
        }) {
            Icon(
                if (terminal.isAgentPane) Icons.Outlined.Terminal else Icons.AutoMirrored.Filled.Chat,
                contentDescription = if (terminal.isAgentPane) "Show the terminal" else "Show the chat",
            )
        }
    }
    var open by remember { mutableStateOf(false) }
    var replacing by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Filled.MoreVert, contentDescription = "More") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(
                text = { Text("Open Worktree") },
                onClick = {
                    open = false
                    onOpenWorktree(TerminalRef(connection.host.id, live.worktreeId, terminal.id))
                },
            )
            if (SeatAction.REPLACE in seatActions(seat, mayControl)) {
                DropdownMenuItem(
                    text = { Text("Replace…") },
                    onClick = {
                        open = false
                        replacing = true
                    },
                )
            }
        }
        HarnessMenu(
            expanded = replacing, availability = availability,
            onDismiss = { replacing = false }, onPick = onReplace,
        )
    }
}

/**
 * The Orchestrator tab with no pane to show (spec §8): none yet, one starting,
 * or one whose pane was lost.
 */
@Composable
private fun OrchestratorEmpty(
    seat: OrchestratorSeat,
    actions: Set<SeatAction>,
    refusal: String?,
    availability: HarnessAvailability,
    runner: String,
    /** The agent a quick exit 127 says isn't installed, when this phone can say which. */
    missing: AgentHarness?,
    onStart: (OrchestratorHarness) -> Unit,
    onReplace: (OrchestratorHarness) -> Unit,
    onRestart: (com.farcooler.model.Terminal) -> Unit,
) {
    Column(
        Modifier.fillMaxSize().padding(32.dp).testTag("orchestrator-empty"),
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        when (seat) {
            is OrchestratorSeat.Empty -> {
                Text(FirstRunCopy.ORCHESTRATOR_TITLE, style = MaterialTheme.typography.titleMedium)
                Text(
                    PhoneEmptyStates.NO_ORCHESTRATOR.lede,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                PhoneEmptyRows(PhoneEmptyStates.NO_ORCHESTRATOR)
                val noneInstalled = availability.isKnown && availability.installed.isEmpty()
                if (noneInstalled) {
                    Text(
                        "No coding agent is installed on $runner. Install Claude Code, Codex, or the Cursor CLI first.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.testTag("orchestrator-none-installed"),
                    )
                }
                if (SeatAction.START in actions) {
                    var picking by remember { mutableStateOf(false) }
                    Box {
                        Button(
                            onClick = { picking = true },
                            enabled = !noneInstalled,
                            modifier = Modifier.testTag("start-orchestrator"),
                        ) {
                            Text(FirstRunCopy.START)
                        }
                        HarnessMenu(
                            expanded = picking, availability = availability,
                            onDismiss = { picking = false }, onPick = onStart,
                        )
                    }
                }
            }
            is OrchestratorSeat.Starting -> {
                CircularProgressIndicator()
                Text("Starting orchestrator…", style = MaterialTheme.typography.titleMedium)
                if (seat.slow) {
                    Text(
                        "This is taking longer than usual.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    if (SeatAction.REPLACE in actions) ReplaceButton(availability, onReplace)
                }
            }
            is OrchestratorSeat.Lost -> if (missing != null) {
                Text(
                    FirstRunCopy.notInstalledTitle(missing),
                    style = MaterialTheme.typography.titleMedium,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.testTag("orchestrator-not-installed"),
                )
                Text(
                    FirstRunCopy.notInstalledBody(missing, runner),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                if (SeatAction.RESTART in actions) {
                    Button(onClick = { onRestart(seat.terminal) }, modifier = Modifier.testTag("orchestrator-try-again")) {
                        Text(FirstRunCopy.TRY_AGAIN)
                    }
                }
            } else {
                Text("The orchestrator stopped", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Restart picks its conversation up where it left off. Replace starts a new one.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                if (SeatAction.RESTART in actions) Button(onClick = { onRestart(seat.terminal) }) { Text("Restart") }
                if (SeatAction.REPLACE in actions) ReplaceButton(availability, onReplace)
            }
            is OrchestratorSeat.Live -> Unit
        }
        refusal?.let {
            Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center)
        }
    }
}

@Composable
private fun ReplaceButton(availability: HarnessAvailability, onReplace: (OrchestratorHarness) -> Unit) {
    var picking by remember { mutableStateOf(false) }
    Box {
        OutlinedButton(onClick = { picking = true }) { Text("Replace…") }
        HarnessMenu(expanded = picking, availability = availability, onDismiss = { picking = false }, onPick = onReplace)
    }
}

/** Claude, Codex or Cursor: what the orchestrator runs on. */
@Composable
private fun HarnessMenu(
    expanded: Boolean,
    availability: HarnessAvailability,
    onDismiss: () -> Unit,
    onPick: (OrchestratorHarness) -> Unit,
) {
    DropdownMenu(expanded = expanded, onDismissRequest = onDismiss) {
        OrchestratorHarness.entries.forEach { harness ->
            val installed = availability.isInstalled(harness.agent)
            DropdownMenuItem(
                text = { Text(if (installed) harness.title else "${harness.title} · ${FirstRunCopy.NOT_INSTALLED}") },
                enabled = installed,
                onClick = {
                    onDismiss()
                    onPick(harness)
                },
            )
        }
    }
}
