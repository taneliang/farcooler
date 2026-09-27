package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
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
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.outlined.AutoAwesome
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.Checklist
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material3.AssistChip
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.coroutineScope
import com.farcooler.model.BoardRow
import com.farcooler.model.GlancePalette
import com.farcooler.model.RunnerBoards
import com.farcooler.model.TaskAcceptanceProgress
import com.farcooler.model.TaskAgentLink
import com.farcooler.model.TaskAgentPresence
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskRow
import com.farcooler.model.Terminal
import com.farcooler.model.Workspace
import com.farcooler.model.landingWorkspace
import com.farcooler.net.Connection
import com.farcooler.net.TerminalRef
import kotlinx.coroutines.launch

// A repository's task board on Android: a Board row on the front door, the
// board itself, and a card opened. Read-and-jump: see the board, see how far
// along each card is, go to the agent on it. Every rule and sentence is in
// `model/TaskBoard.kt`, the Android twin of AgentKit's, so a card reads the
// same here as on the Mac and the iPhone.

/**
 * One runner's Board rows, for the front door: a row per repository whose board
 * has something on it. Collected here rather than in the screen because each
 * runner's boards are their own flows, and a runner that drops keeps its rows
 * as last read — its agent count goes quiet, the row does not vanish.
 */
@Composable
fun RunnerBoardRows(
    connection: Connection,
    namesRunner: Boolean,
    onOpen: (BoardRow) -> Unit,
) {
    val repositories by connection.repositories.collectAsStateWithLifecycle()
    val boards by connection.boards.collectAsStateWithLifecycle()
    val fleet by connection.fleet.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val phase by connection.phase.collectAsStateWithLifecycle()

    val rows = RunnerBoards.rows(
        hostId = connection.host.id,
        repositories = repositories,
        boards = boards,
        panes = fleet.workspaces.flatMap { it.terminals },
        build = daemon,
        connected = phase is Connection.Phase.Connected,
    )
    Column {
        rows.forEach { row -> BoardRowItem(row, if (namesRunner) connection.host.label else null, onOpen) }
    }
}

/** One Board row: the repository, a quiet count of tasks with agents, and the decisions in amber. */
@Composable
private fun BoardRowItem(row: BoardRow, runner: String?, onOpen: (BoardRow) -> Unit) {
    val amber = glanceColor(GlancePalette.amber)
    ListItem(
        headlineContent = { Text("${row.name} board", maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = runner?.let { { Text(it) } },
        leadingContent = {
            Icon(
                Icons.Outlined.Checklist,
                contentDescription = null,
                modifier = Modifier.size(20.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        trailingContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (row.agents > 0) {
                    Icon(
                        Icons.Outlined.AutoAwesome,
                        contentDescription = null,
                        modifier = Modifier.size(14.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.width(2.dp))
                    Text(
                        "${row.agents}",
                        style = MaterialTheme.typography.labelLarge,
                        fontFamily = FontFamily.Monospace,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.width(12.dp))
                }
                if (row.decisions > 0) {
                    Text(
                        "${row.decisions}",
                        style = MaterialTheme.typography.labelLarge,
                        fontFamily = FontFamily.Monospace,
                        fontWeight = FontWeight.SemiBold,
                        color = amber,
                    )
                }
            }
        },
        modifier = Modifier
            .clickable { onOpen(row) }
            .testTag("board-row-${row.hostId}-${row.repository}")
            .semantics {
                contentDescription = listOfNotNull("${row.name} board", row.spoken).joinToString(", ")
            },
    )
}

/** The panes working [row] on one runner, named for a menu: the pane, then its workspace. */
fun boardAgents(row: TaskRow, workspaces: List<Workspace>): List<Pair<Terminal, String>> {
    val found = workspaces.flatMap { workspace ->
        val ordinals = workspace.ordinals()
        row.livePanes(workspace.terminals).map { terminal ->
            terminal to "${terminal.displayName(ordinals[terminal.id])} in ${workspace.task}"
        }
    }
    val titles = TaskAgentLink.menuTitles(found.map { it.second }, found.map { it.first.short })
    return found.map { it.first }.zip(titles)
}

/**
 * One repository's board: a list grouped by status, Needs Decision first, and
 * only the statuses with something in them.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BoardScreen(
    connection: Connection,
    repository: String,
    onOpenTask: (taskId: String) -> Unit,
    onJump: (TerminalRef) -> Unit,
    onBack: () -> Unit,
) {
    val repositories by connection.repositories.collectAsStateWithLifecycle()
    val boards by connection.boards.collectAsStateWithLifecycle()
    val unread by connection.unreadBoards.collectAsStateWithLifecycle()
    val fleet by connection.fleet.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val phase by connection.phase.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    var refreshing by remember { mutableStateOf(false) }

    // Read on opening, whatever was last read: the row that opened this may
    // be showing a count from before the last reconnect.
    LaunchedEffect(repository) { connection.readBoard(repository) }

    // The row's own fallback, so the board is titled what its row was.
    val name = repositories.firstOrNull { it.id == repository }
        ?.let { it.displayName.ifEmpty { it.short } } ?: "Board"
    val board = boards[repository]
    val speaks = TaskAgentLink.speaksOfAgents(phase is Connection.Phase.Connected, daemon)
    val jump = boardJump(connection.host.id, fleet.workspaces, onJump) { why ->
        scope.launch { snackbar.showSnackbar(why) }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        val waiting = board?.let { TaskBoard.waitingSentence(it.waitingOnYou) }
                        if (waiting != null) {
                            Text(
                                waiting,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    connection.readBoard(repository)
                    refreshing = false
                }
            },
            modifier = Modifier.fillMaxSize().padding(padding),
        ) {
            when {
                board == null && repository in unread -> Empty(
                    "Couldn’t read this board",
                    "Far Cooler couldn’t read this board. Pull down to try again.",
                )
                board == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator()
                }
                board.isEmpty -> Empty("No tasks", "Nothing is on this board yet.")
                else -> LazyColumn(Modifier.fillMaxSize().testTag("board")) {
                    if (repository in unread) {
                        item(key = "unread") {
                            ListItem(
                                headlineContent = {
                                    Text("Couldn’t read this board just now. This is how it was last read.")
                                },
                                leadingContent = { Icon(Icons.Outlined.Warning, contentDescription = null) },
                            )
                        }
                    }
                    board.listed.forEach { column ->
                        item(key = "header/${column.status.wire}") {
                            Text(
                                "${column.status.title}  ${column.rows.size}",
                                style = MaterialTheme.typography.titleSmall,
                                color = MaterialTheme.colorScheme.primary,
                                modifier = Modifier
                                    .padding(start = 16.dp, top = 20.dp, bottom = 4.dp)
                                    .testTag("board-section-${column.status.wire}"),
                            )
                        }
                        items(column.rows, key = { "task/${it.id}" }) { row ->
                            val agents = if (speaks) boardAgents(row, fleet.workspaces) else emptyList()
                            TaskCardRow(
                                row = row,
                                agents = agents,
                                presence = row.agentPresence(agents.size, speaks),
                                onOpen = { onOpenTask(row.id) },
                                onJump = jump,
                            )
                            HorizontalDivider()
                        }
                    }
                    if (board.unreadable.isNotEmpty()) {
                        item(key = "unreadable") {
                            Text(
                                "Not on this version",
                                style = MaterialTheme.typography.titleSmall,
                                modifier = Modifier.padding(start = 16.dp, top = 20.dp, bottom = 4.dp),
                            )
                        }
                        items(board.unreadable, key = { "unreadable/${it.id}" }) { row ->
                            ListItem(
                                overlineContent = { Text(row.key, fontFamily = FontFamily.Monospace) },
                                headlineContent = { Text(row.title) },
                                supportingContent = { Text(row.status, fontFamily = FontFamily.Monospace) },
                            )
                        }
                    }
                }
            }
        }
    }
}

/**
 * An Agent tap: land on the pane if this runner's fleet still has it, or say
 * why not and stay on the board. Resolved at the tap, against the fleet as it
 * is now, because the card was drawn from the fleet as it was.
 */
private fun boardJump(
    hostId: String,
    workspaces: List<Workspace>,
    onJump: (TerminalRef) -> Unit,
    onRefused: (String) -> Unit,
): (Terminal) -> Unit = { terminal ->
    val workspace = landingWorkspace(terminal.id, workspaces)
    if (workspace == null) onRefused(TaskAgentLink.PANE_HAS_CLOSED)
    else onJump(TerminalRef(hostId, workspace, terminal.id))
}

@Composable
private fun Empty(title: String, detail: String) {
    Column(
        Modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(title, style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.size(8.dp))
        Text(detail, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** One card: key, title, what it asks, how long it has sat, acceptance, and its agent. */
@Composable
private fun TaskCardRow(
    row: TaskRow,
    agents: List<Pair<Terminal, String>>,
    presence: TaskAgentPresence,
    onOpen: () -> Unit,
    onJump: (Terminal) -> Unit,
) {
    val now = rememberMinuteClock()
    ListItem(
        overlineContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(row.key, fontFamily = FontFamily.Monospace)
                if (row.isStale(now)) {
                    Spacer(Modifier.width(4.dp))
                    Icon(Icons.Outlined.Schedule, contentDescription = null, modifier = Modifier.size(12.dp))
                }
            }
        },
        headlineContent = { Text(row.title, maxLines = 3, overflow = TextOverflow.Ellipsis) },
        supportingContent = { CardDetails(row, now) },
        trailingContent = { AgentControl(row.key, agents, presence, onJump) },
        modifier = Modifier
            .clickable(onClick = onOpen)
            .testTag("board-card-${row.key}"),
    )
}

/**
 * The wall clock, as of the last minute boundary, and moving on each one.
 *
 * A [MinuteClock] per card on screen, asleep between minutes, started on
 * ON_START and stopped on ON_STOP (and with the card). ON_START runs before
 * the first frame the app draws when it comes back or the phone wakes, and
 * [MinuteClock.start] has read the clock before it returns, so that frame is
 * right. Started, not resumed: a board half-covered by another window is
 * still on screen, and its cards still move on.
 */
@Composable
private fun rememberMinuteClock(): Long {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    val clock = remember { MinuteClock(System::currentTimeMillis) { now = it } }
    LifecycleStartEffect(clock) {
        clock.start(lifecycle.coroutineScope)
        onStopOrDispose { clock.stop() }
    }
    return now
}

@Composable
private fun CardDetails(row: TaskRow, now: Long) {
    val amber = glanceColor(GlancePalette.amber)
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        row.callToAction?.let {
            Text(it, color = amber, style = MaterialTheme.typography.labelMedium)
        }
        row.stalenessNote(now)?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
        // "Updated 2h ago" or "Added 3d ago"; null on a stale card, whose
        // sentence above already says how long.
        row.timeNote(now)?.let {
            Text(
                it,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        row.acceptanceProgress?.let { AcceptanceLine(it) }
    }
}

/** `2 of 5`, or `All 5 met` in the accent color once every line holds. */
@Composable
private fun AcceptanceLine(progress: TaskAcceptanceProgress) {
    val color = if (progress.isComplete) MaterialTheme.colorScheme.primary
    else MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.semantics { contentDescription = "Acceptance: ${progress.sentence}" },
    ) {
        Icon(
            if (progress.isComplete) Icons.Outlined.CheckCircle else Icons.Outlined.CheckCircleOutline,
            contentDescription = null,
            modifier = Modifier.size(14.dp),
            tint = color,
        )
        Spacer(Modifier.width(4.dp))
        Text(progress.sentence, style = MaterialTheme.typography.bodySmall, color = color)
    }
}

/** A chip for one agent, a chip with a menu for several, a quiet "No Agent", or nothing. */
@Composable
private fun AgentControl(
    key: String,
    agents: List<Pair<Terminal, String>>,
    presence: TaskAgentPresence,
    onJump: (Terminal) -> Unit,
) {
    when (presence) {
        TaskAgentPresence.Unsaid -> Unit
        TaskAgentPresence.NoAgent -> Text(
            presence.title.orEmpty(),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.testTag("board-no-agent-$key"),
        )
        is TaskAgentPresence.Agents -> {
            var open by remember { mutableStateOf(false) }
            Box {
                AssistChip(
                    onClick = {
                        val only = agents.singleOrNull()
                        if (only != null) onJump(only.first) else open = true
                    },
                    label = { Text(presence.title.orEmpty()) },
                    trailingIcon = {
                        Icon(
                            if (agents.size == 1) Icons.AutoMirrored.Filled.ArrowForward
                            else Icons.Outlined.KeyboardArrowDown,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                    },
                    modifier = Modifier.testTag("board-agent-$key"),
                )
                DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                    agents.forEach { (terminal, title) ->
                        DropdownMenuItem(
                            text = { Text(title) },
                            onClick = {
                                open = false
                                onJump(terminal)
                            },
                        )
                    }
                }
            }
        }
    }
}

/**
 * One card, opened: key, title, status, what it asks, intent, every acceptance
 * line with its tick, labels, and the same Agent control. Read-only; everything
 * here came with `task.list`, so opening a card costs no round trip.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TaskDetailScreen(
    connection: Connection,
    repository: String,
    taskId: String,
    onJump: (TerminalRef) -> Unit,
    onBack: () -> Unit,
) {
    val boards by connection.boards.collectAsStateWithLifecycle()
    val fleet by connection.fleet.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val phase by connection.phase.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }

    val row = boards[repository]?.row(taskId)
    val speaks = TaskAgentLink.speaksOfAgents(phase is Connection.Phase.Connected, daemon)
    val jump = boardJump(connection.host.id, fleet.workspaces, onJump) { why ->
        scope.launch { snackbar.showSnackbar(why) }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(row?.key ?: "Task", fontFamily = FontFamily.Monospace) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        if (row == null) {
            Box(Modifier.padding(padding)) {
                Empty("Not on this board", "This task isn’t on the board anymore.")
            }
            return@Scaffold
        }
        val agents = if (speaks) boardAgents(row, fleet.workspaces) else emptyList()
        val presence = row.agentPresence(agents.size, speaks)
        LazyColumn(Modifier.fillMaxSize().padding(padding).testTag("board-detail")) {
            item(key = "heading") {
                Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(row.title, style = MaterialTheme.typography.titleLarge)
                    Text(
                        row.status.title,
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    CardDetails(row.copy(acceptance = emptyList()), rememberMinuteClock())
                }
            }
            if (presence != TaskAgentPresence.Unsaid) {
                item(key = "agent") {
                    ListItem(
                        headlineContent = { Text("Agent") },
                        trailingContent = { AgentControl(row.key, agents, presence, jump) },
                    )
                }
            }
            if (row.intent.isNotEmpty()) {
                item(key = "intent") {
                    Section("Intent")
                    Text(
                        row.intent,
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.padding(horizontal = 16.dp).testTag("board-detail-intent"),
                    )
                }
            }
            row.acceptanceProgress?.let { progress ->
                item(key = "acceptance-header") { Section("Acceptance · ${progress.sentence}") }
                items(row.acceptance, key = { "line/${it.id}" }) { line ->
                    ListItem(
                        leadingContent = {
                            Icon(
                                if (line.met) Icons.Outlined.CheckCircle else Icons.Outlined.RadioButtonUnchecked,
                                contentDescription = null,
                                tint = if (line.met) MaterialTheme.colorScheme.primary
                                else MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                        headlineContent = { Text(line.text) },
                        modifier = Modifier
                            .testTag("board-acceptance-${line.id}")
                            .semantics { contentDescription = "${line.text}. ${if (line.met) "Met" else "Not met"}" },
                    )
                }
            }
            if (row.labels.isNotEmpty()) {
                item(key = "labels") {
                    Section("Labels")
                    Text(
                        row.labels.joinToString(", "),
                        fontFamily = FontFamily.Monospace,
                        modifier = Modifier.padding(horizontal = 16.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun Section(title: String) {
    Text(
        title,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(start = 16.dp, top = 20.dp, bottom = 4.dp),
    )
}
