package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.ui.text.style.TextDecoration
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
import com.farcooler.model.Worktree
import com.farcooler.model.landingWorktree
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Difference
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.TextButton
import androidx.compose.runtime.saveable.rememberSaveable
import com.farcooler.core.CoreException
import com.farcooler.model.NeedsYouAnswer
import com.farcooler.model.NeedsYouRow
import com.farcooler.model.RunnerNeedsYouItem
import com.farcooler.model.TaskStatus
import com.farcooler.model.WorkspaceSummary
import com.farcooler.model.NewTask
import com.farcooler.core.refusalWord
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.Button
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import com.farcooler.net.rethrowIfCancellation
import com.farcooler.net.Connection
import com.farcooler.net.TerminalRef
import kotlinx.coroutines.launch

// A workspace's task board on Android: the Board tab of its workspace screen,
// and a card opened. Every rule and sentence is in `model/TaskBoard.kt`, the
// Android twin of AgentKit's, so a card reads the same here as on the Mac and
// the iPhone; the list form's rows are `BoardList`'s.

/** The panes working [row] on one runner, named for a menu: the pane, then its worktree. */
fun boardAgents(row: TaskRow, worktrees: List<Worktree>): List<Pair<Terminal, String>> {
    val found = worktrees.flatMap { worktree ->
        val ordinals = worktree.ordinals()
        row.livePanes(worktree.terminals).map { terminal ->
            terminal to "${terminal.displayName(ordinals[terminal.id])} in ${worktree.task}"
        }
    }
    val titles = TaskAgentLink.menuTitles(found.map { it.second }, found.map { it.first.short })
    return found.map { it.first }.zip(titles)
}

/**
 * A workspace's Board tab: the list form of spec §5, from
 * [TaskBoard.sections] — every status, Needs Decision first, an empty one a
 * collapsed header reading "Backlog 0", Done and Canceled collapsed until
 * opened. On a runner without workspaces the workspace is its repository's
 * implicit one, and the board the repository's.
 *
 * A card opens the task screen over the workspace; its Agent control opens
 * the agent over that, so Back comes out onto the board it was chosen from.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BoardTab(
    connection: Connection,
    workspace: WorkspaceSummary,
    onOpenTask: (taskId: String) -> Unit,
    onJump: (TerminalRef) -> Unit,
    modifier: Modifier = Modifier,
) {
    val boards by connection.boards.collectAsStateWithLifecycle()
    val unread by connection.unreadBoards.collectAsStateWithLifecycle()
    val fleet by connection.fleet.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val link by connection.link.collectAsStateWithLifecycle()
    val needsYou by connection.needsYou.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }
    var refreshing by remember { mutableStateOf(false) }
    // The statuses flipped from their first state, per workspace, for as long
    // as the screen's saved state is kept.
    var toggled by rememberSaveable(workspace.id) { mutableStateOf(emptyList<String>()) }
    // Done draws its recent cards until this is asked for (BoardDone).
    var showAllDone by rememberSaveable(workspace.id) { mutableStateOf(false) }

    // Read on opening, whatever was last read: the row that opened this may
    // be showing a count from before the last reconnect. While it is open, a
    // reconnect's sweep reads it again (`Connection.loadBoardsDetached`), and
    // so does a notice naming this workspace.
    LaunchedEffect(workspace.id) { connection.readBoard(workspace) }

    val board = boards[workspace.id]
    val speaks = TaskAgentLink.speaksOfAgents(link, daemon)
    val jump = boardJump(connection.host.id, fleet.worktrees, onJump) { why ->
        scope.launch { snackbar.showSnackbar(why) }
    }
    val flipped = toggled.mapNotNull(TaskStatus::parse).toSet()
    // New Task…: a Control-scope write, so not on a read-scoped connection.
    val offersNewTask = NewTask.offered(daemon)
    var composing by rememberSaveable(workspace.id) { mutableStateOf(false) }

    Box(modifier.fillMaxSize()) {
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    connection.readBoard(workspace)
                    refreshing = false
                }
            },
            modifier = Modifier.fillMaxSize(),
        ) {
            when {
                board == null && workspace.id in unread -> Empty(
                    "Couldn’t read this board",
                    "Far Cooler couldn’t read this board. Pull down to try again.",
                )
                board == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator()
                }
                else -> LazyColumn(
                    Modifier.fillMaxSize().testTag("board"),
                    // Room below the last card for New Task…, so it never sits on one.
                    contentPadding = PaddingValues(bottom = if (offersNewTask) 88.dp else 0.dp),
                ) {
                    if (workspace.id in unread) {
                        item(key = "unread") {
                            ListItem(
                                headlineContent = {
                                    Text("Couldn’t read this board just now. This is how it was last read.")
                                },
                                leadingContent = { Icon(Icons.Outlined.Warning, contentDescription = null) },
                            )
                        }
                    }
                    // The workspace's decision items once the runner's list is
                    // read, the Needs Decision column until then (spec §2.2).
                    val waitingCount = RunnerBoards.waiting(
                        columnCount = board.waitingOnYou,
                        decisions = RunnerBoards.decisions(workspace, needsYou?.items.orEmpty()),
                        listRead = needsYou != null,
                        listServed = daemon?.can("needs_you") == true,
                    )
                    TaskBoard.waitingSentence(waitingCount)?.let { waiting ->
                        item(key = "waiting") {
                            Text(
                                waiting,
                                style = MaterialTheme.typography.bodySmall,
                                color = glanceColor(GlancePalette.amber),
                                modifier = Modifier.padding(start = 16.dp, top = 12.dp),
                            )
                        }
                    }
                    for (entry in BoardList.entries(board, flipped, showAllDone = showAllDone)) {
                        when (entry) {
                            is BoardListEntry.Header -> item(key = entry.key) {
                                SectionHeader(entry) {
                                    val word = entry.status.wire
                                    toggled = if (word in toggled) toggled - word else toggled + word
                                }
                            }
                            is BoardListEntry.ShowAllDone -> item(key = entry.key) {
                                TextButton(
                                    onClick = { showAllDone = !showAllDone },
                                    modifier = Modifier.padding(start = 8.dp).testTag("board-show-all-done"),
                                ) { Text(entry.title) }
                            }
                            is BoardListEntry.Card -> item(key = entry.key) {
                                val row = entry.row
                                val agents = if (speaks) boardAgents(row, fleet.worktrees) else emptyList()
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
        if (offersNewTask) {
            ExtendedFloatingActionButton(
                onClick = { composing = true },
                icon = { Icon(Icons.Filled.Add, contentDescription = null) },
                text = { Text("New Task…") },
                modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp).testTag("board-new-task"),
            )
        }
        // Above the New Task… button, which sits at the bottom end.
        SnackbarHost(
            snackbar,
            Modifier.align(Alignment.BottomCenter).padding(bottom = if (offersNewTask) 72.dp else 0.dp),
        )
    }
    if (composing && offersNewTask) {
        NewTaskSheet(connection, workspace, onDismiss = { composing = false })
    }
}

/**
 * New Task…: a title, which it needs, and details, which become the task's
 * intent. Files on [workspace]'s board, as the person holding the phone.
 *
 * A failed create keeps what was typed and says why under it, in this app's
 * words (`NewTask.refusal`); only a filed task closes the sheet.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NewTaskSheet(connection: Connection, workspace: WorkspaceSummary, onDismiss: () -> Unit) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    var title by rememberSaveable { mutableStateOf("") }
    var details by rememberSaveable { mutableStateOf("") }
    // Saved with the rest: a create in flight when the activity is recreated may
    // have landed, and an enabled button under it would file a second one. Its
    // coroutine dies with the old activity, so nothing here would ever clear
    // it; the effect below turns a restored "sending" into a sentence instead.
    var sending by rememberSaveable { mutableStateOf(false) }
    var failure by rememberSaveable { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        if (sending) {
            sending = false
            failure = "Couldn’t tell whether that task was added. Check the board before trying again."
        }
    }
    val trimmed = title.trim()
    val tooLong = trimmed.isNotEmpty() && !NewTask.titleFits(trimmed)

    ModalBottomSheet(onDismissRequest = { if (!sending) onDismiss() }, sheetState = state) {
        Column(
            Modifier
                .padding(horizontal = 20.dp)
                .padding(bottom = 20.dp)
                .imePadding()
                .navigationBarsPadding()
                .testTag("new-task-sheet"),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("New Task", style = MaterialTheme.typography.headlineSmall)
            OutlinedTextField(
                value = title,
                onValueChange = { title = it },
                label = { Text("Title") },
                singleLine = true,
                enabled = !sending,
                isError = tooLong,
                modifier = Modifier.fillMaxWidth().testTag("new-task-title"),
            )
            if (tooLong) {
                Text(
                    NewTask.refusal(null, "title"),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
            OutlinedTextField(
                value = details,
                onValueChange = { details = it },
                label = { Text("Details (optional)") },
                minLines = 3,
                enabled = !sending,
                modifier = Modifier.fillMaxWidth().testTag("new-task-details"),
            )
            failure?.let {
                Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            }
            Button(
                onClick = {
                    sending = true
                    failure = null
                    scope.launch {
                        try {
                            connection.createTask(workspace, title, details)
                            onDismiss()
                        } catch (e: Exception) {
                            e.rethrowIfCancellation()
                            failure = NewTask.refusal(e.refusalWord, (e as? CoreException)?.what)
                        } finally {
                            sending = false
                        }
                    }
                },
                enabled = !sending && NewTask.titleFits(title),
                modifier = Modifier.fillMaxWidth().testTag("new-task-add"),
            ) {
                if (sending) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                else Text("Add Task")
            }
        }
    }
}

/**
 * A status's header: its title and count, and a chevron when it has tasks to
 * show or hide. An empty one says 0 and does nothing when tapped.
 */
@Composable
private fun SectionHeader(header: BoardListEntry.Header, onToggle: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .fillMaxWidth()
            .then(if (header.expandable) Modifier.clickable(onClick = onToggle) else Modifier)
            .heightIn(min = 48.dp)
            .padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp)
            .testTag("board-section-${header.status.wire}")
            .semantics(mergeDescendants = true) {
                contentDescription = buildString {
                    append("${header.status.title}, ${header.count}")
                    if (header.expandable) append(if (header.expanded) ", expanded" else ", collapsed")
                }
            },
    ) {
        Text(
            header.status.title,
            style = MaterialTheme.typography.titleSmall,
            color = if (header.count > 0) MaterialTheme.colorScheme.primary
            else MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.width(8.dp))
        Text(
            "${header.count}",
            style = MaterialTheme.typography.labelMedium,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.weight(1f))
        if (header.expandable) {
            Icon(
                if (header.expanded) Icons.Outlined.KeyboardArrowDown
                else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                contentDescription = null,
                modifier = Modifier.size(18.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
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
    worktrees: List<Worktree>,
    onJump: (TerminalRef) -> Unit,
    onRefused: (String) -> Unit,
): (Terminal) -> Unit = { terminal ->
    val worktree = landingWorktree(terminal.id, worktrees)
    if (worktree == null) onRefused(TaskAgentLink.PANE_HAS_CLOSED)
    else onJump(TerminalRef(hostId, worktree, terminal.id))
}

@Composable
private fun Empty(title: String, detail: String) {
    EmptyState(title, detail, Modifier.fillMaxSize())
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
 *
 * [wallClock] is for `MinuteClockLifecycleTest` alone, which runs this under a
 * lifecycle it drives, on a clock it moves. Every caller here takes the
 * default.
 */
@Composable
internal fun rememberMinuteClock(wallClock: () -> Long = System::currentTimeMillis): Long {
    var now by remember { mutableLongStateOf(wallClock()) }
    val clock = remember { MinuteClock(wallClock) { now = it } }
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

/** A chip for one agent, a chip with a menu for several, a quiet "No agent", or nothing. */
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
    workspaceId: String,
    taskId: String,
    onJump: (TerminalRef) -> Unit,
    /** Its worktree, pushed over this task: on its Changes tab when [changes]. */
    onOpenWorktree: (worktreeId: String, changes: Boolean) -> Unit,
    onBack: () -> Unit,
) {
    val boards by connection.boards.collectAsStateWithLifecycle()
    val unread by connection.unreadBoards.collectAsStateWithLifecycle()
    val fleet by connection.fleet.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val link by connection.link.collectAsStateWithLifecycle()
    val needsYou by connection.needsYou.collectAsStateWithLifecycle()
    val inbox by connection.inbox.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val snackbar = remember { SnackbarHostState() }

    // Reached from Needs You or a notification, the board may not have been
    // read on this link yet: read it rather than say the task has gone.
    LaunchedEffect(workspaceId) {
        if (connection.boards.value[workspaceId] == null) connection.readBoard(connection.board(workspaceId))
    }

    val row = boards[workspaceId]?.row(taskId)
    // What this task asks of you, as the runner put it: a decision with its
    // options, a review, or an ask its agent holds. Answered here as on Needs
    // You, by the same row.
    val asking = needsYou?.items?.firstOrNull { it.task?.id == taskId }
    val mayAnswer = daemon?.grantedScope != "read"
    val speaks = TaskAgentLink.speaksOfAgents(link, daemon)
    val jump = boardJump(connection.host.id, fleet.worktrees, onJump) { why ->
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
                if (boards[workspaceId] == null && workspaceId !in unread) {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                } else {
                    Empty("Not on this board", "This task isn’t on the board anymore.")
                }
            }
            return@Scaffold
        }
        val agents = if (speaks) boardAgents(row, fleet.worktrees) else emptyList()
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
            if (asking != null) {
                item(key = "asking") {
                    NeedsYouItemRow(
                        row = NeedsYouRow(RunnerNeedsYouItem(connection.host.id, asking), place = "", runner = null),
                        connection = connection,
                        showPlace = false,
                        // Open from here is the agent it's about, if any: the
                        // task is already on screen.
                        onOpen = {
                            asking.terminal?.id?.let { id ->
                                fleet.worktrees.firstOrNull { w -> w.terminals.any { it.id == id } }
                                    ?.terminals?.firstOrNull { it.id == id }?.let(jump)
                            }
                        },
                    )
                }
            } else if (row.status == TaskStatus.NEEDS_DECISION && mayAnswer) {
                // A runner too old to list its decisions: the answer is still
                // a note, and still written here.
                item(key = "answer") {
                    AnswerDecision(connection, row)
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
            // Its worktree and changes, through the task's own worktree_id:
            // reachable whether or not an agent is still on it (spec §3.2).
            items(TaskLinks.rows(row, fleet.worktrees), key = { "link/${it::class.simpleName}" }) { link ->
                when (link) {
                    is TaskLinkRow.Changes -> {
                        val counts = inbox[link.worktreeId]
                        ListItem(
                            headlineContent = { Text("Changes") },
                            leadingContent = { Icon(Icons.Outlined.Difference, contentDescription = null) },
                            trailingContent = { if (counts != null && counts.hasDiff) DiffCounts(counts) },
                            modifier = Modifier
                                .clickable { onOpenWorktree(link.worktreeId, true) }
                                .testTag("task-changes"),
                        )
                    }
                    is TaskLinkRow.Worktree -> ListItem(
                        headlineContent = { Text("Worktree") },
                        supportingContent = { Text(link.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        leadingContent = { Icon(Icons.Outlined.Folder, contentDescription = null) },
                        modifier = Modifier
                            .clickable { onOpenWorktree(link.worktreeId, false) }
                            .testTag("task-worktree"),
                    )
                }
            }
            if (row.intent.isNotEmpty()) {
                item(key = "intent") {
                    Section("Intent")
                    // Markdown, as the Mac and iOS draw it (ov-98).
                    MarkdownText(
                        row.intent,
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
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                        // Its inline Markdown; met, struck through and quiet,
                        // as on the Mac (ov-98).
                        headlineContent = {
                            Text(
                                inline(line.text),
                                color = if (line.met) MaterialTheme.colorScheme.onSurfaceVariant
                                else MaterialTheme.colorScheme.onSurface,
                                textDecoration = if (line.met) TextDecoration.LineThrough else null,
                            )
                        },
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

/**
 * "Answer…" on a task in Needs Decision whose runner didn't list the
 * decision: a typed answer, sent as an `answer` note.
 */
@Composable
private fun AnswerDecision(connection: Connection, row: TaskRow) {
    val scope = rememberCoroutineScope()
    var writing by remember { mutableStateOf(false) }
    var sending by remember { mutableStateOf(false) }
    var refusal by remember { mutableStateOf<String?>(null) }
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
        if (sending) {
            CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
        } else {
            OutlinedButton(onClick = { writing = true }) { Text("Answer…") }
        }
        refusal?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }
    }
    if (writing) {
        var text by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { writing = false },
            title = { Text("Answer ${row.key}") },
            text = { OutlinedTextField(value = text, onValueChange = { text = it }, minLines = 2) },
            confirmButton = {
                TextButton(enabled = text.isNotBlank(), onClick = {
                    writing = false
                    sending = true
                    refusal = null
                    scope.launch {
                        try {
                            connection.answerDecision(row.id, text.trim())
                        } catch (e: Exception) {
                            e.rethrowIfCancellation()
                            refusal = NeedsYouAnswer.refusal((e as? CoreException)?.what, "the runner")
                        } finally {
                            sending = false
                        }
                    }
                }) { Text("Send") }
            },
            dismissButton = { TextButton(onClick = { writing = false }) { Text("Cancel") } },
        )
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
