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
import androidx.compose.foundation.layout.widthIn
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
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.coroutineScope
import com.farcooler.model.BoardReads
import com.farcooler.model.BoardRow
import com.farcooler.model.BoardSummary
import com.farcooler.model.Capability
import com.farcooler.model.FirstRunCopy
import com.farcooler.model.GlancePalette
import com.farcooler.model.PhoneFirstRun
import com.farcooler.model.Markdown
import com.farcooler.model.RunnerBoards
import com.farcooler.model.AskAboutTask
import com.farcooler.model.TaskAcceptanceProgress
import com.farcooler.model.TaskUsageState
import com.farcooler.model.TaskAgentLink
import com.farcooler.model.blockedSummary
import com.farcooler.model.orchestratorTerminalId
import com.farcooler.model.startLine
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
 * The pane [row]'s subagents live in: the orchestrator's, which is where a
 * Subagent control goes (ov-213). Null when the runner did not say which pane,
 * or it has closed since, so the control is never a button that leads nowhere.
 */
fun boardOrchestrator(row: TaskRow, worktrees: List<Worktree>): Pair<Terminal, String>? {
    val id = row.orchestratorTerminalId ?: return null
    for (worktree in worktrees) {
        val terminal = worktree.terminals.firstOrNull { it.id == id && it.state in TaskAgentLink.LIVE_STATES }
            ?: continue
        return terminal to "Orchestrator in ${worktree.task}"
    }
    return null
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
    /** A finished status's History page (ov-103). */
    onOpenHistory: (TaskStatus) -> Unit = {},
    /** A theme's or lane's page of the plan layer (ov-274). */
    onOpenPlan: (com.farcooler.model.PlanPage) -> Unit = {},
    /** Whether an orchestrator is up to tell, which decides what a blank board says (ov-205). */
    orchestratorRunning: Boolean = true,
    /** Switches to the Orchestrator tab, for a blank board with none running. */
    onShowOrchestrator: (() -> Unit)? = null,
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
    // The long sections showing every task, not just ten.
    var showingMore by rememberSaveable(workspace.id) { mutableStateOf(emptyList<String>()) }
    // What's been read on this board (ov-104, ov-113): the runner's state when it
    // keeps it, this phone's own when it can't. What Unread lists and Done keeps.
    val allReads by connection.readState.collectAsStateWithLifecycle()
    val reads = allReads[workspace.id] ?: BoardReads.firstLook(System.currentTimeMillis())
    var askingMarkAll by remember { mutableStateOf(false) }
    // The Tasks | Plan choice (ov-274), kept per board on this phone. Tasks until chosen otherwise.
    val planPrefs = androidx.compose.ui.platform.LocalContext.current.getSharedPreferences("farcooler.plan", android.content.Context.MODE_PRIVATE)
    val planKey = com.farcooler.model.PlanChoice.key(connection.host.id, workspace.id)
    var planChosen by remember(planKey) { mutableStateOf(planPrefs.getBoolean(planKey, false)) }
    val planStates by connection.plans.states.collectAsStateWithLifecycle()
    val keepsPlan = daemon?.can(Capability.BOARD_PLAN) == true
    val showsPlan = com.farcooler.model.PlanChoice.showing(keepsPlan, planChosen)
    var landedOpen by rememberSaveable(workspace.id) { mutableStateOf(false) }
    LaunchedEffect(showsPlan, workspace.id) { if (showsPlan) connection.plans.read(workspace) }

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
    val notes = board?.let { rememberUnreadNotes(connection, it, reads) } ?: emptyMap()
    val summary = board?.let { BoardSummary.make(it.rows, notes, reads) }

    Box(modifier.fillMaxSize()) {
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    connection.readBoard(workspace)
                    if (showsPlan) connection.plans.read(workspace)
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
                // Seven headers each reading zero is a blank page. Show the shape of
                // what will appear, say who fills it and, with no orchestrator to
                // tell, how to start one: the orchestrator owns the task list.
                board.isEmpty && workspace.id !in unread -> {
                    BoardBlank(!workspace.isImplicit, orchestratorRunning, onShowOrchestrator)
                }
                else -> LazyColumn(Modifier.fillMaxSize().testTag("board")) {
                    if (keepsPlan) {
                        item(key = "plan/switch") {
                            PlanSwitch(showsPlan, onChange = {
                                planChosen = it
                                planPrefs.edit().putBoolean(planKey, it).apply()
                            })
                        }
                    }
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
                        listServed = daemon?.can(Capability.NEEDS_YOU) == true,
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
                    val more = showingMore.mapNotNull(TaskStatus::parse).toSet()
                    val entries = BoardList.entries(board, flipped, reads = reads, showingMore = more, unread = summary, showsPlan = showsPlan)
                    for (entry in entries) {
                        when (entry) {
                            is BoardListEntry.UnreadHeader -> item(key = entry.key) {
                                UnreadHeader(entry) { askingMarkAll = true }
                            }
                            is BoardListEntry.UnreadNothing -> item(key = entry.key) { UnreadNothingRow(entry) }
                            is BoardListEntry.UnreadGroup -> item(key = entry.key) { UnreadGroupRow(entry) }
                            is BoardListEntry.UnreadLine -> item(key = entry.key) {
                                UnreadLineRow(entry.item.key, entry.item.title, "board-unread-${entry.item.id}", entry.item.whenSaid(System.currentTimeMillis())) {
                                    connection.readsSync.open(workspace.id, board.row(entry.item.taskId) ?: return@UnreadLineRow)
                                    onOpenTask(entry.item.taskId)
                                }
                            }
                            is BoardListEntry.UnreadNote -> item(key = entry.key) {
                                UnreadNoteRow(entry.activity, "board-unread-activity-${entry.activity.key}") {
                                    connection.readsSync.open(workspace.id, board.row(entry.activity.taskId) ?: return@UnreadNoteRow)
                                    onOpenTask(entry.activity.taskId)
                                }
                            }
                            is BoardListEntry.UnreadMore -> item(key = entry.key) { UnreadMoreRow(entry) }
                            is BoardListEntry.Header -> item(key = entry.key) {
                                SectionHeader(entry) {
                                    val word = entry.status.wire
                                    toggled = if (word in toggled) toggled - word else toggled + word
                                }
                            }
                            is BoardListEntry.ShowMore -> item(key = entry.key) {
                                TextButton(
                                    onClick = {
                                        val word = entry.status.wire
                                        showingMore = if (entry.showingAll) showingMore - word else showingMore + word
                                    },
                                    modifier = Modifier.padding(start = 8.dp).testTag("board-show-more-${entry.status.wire}"),
                                ) { Text(entry.title) }
                            }
                            is BoardListEntry.History -> item(key = entry.key) {
                                HistoryRow(entry) { onOpenHistory(entry.status) }
                            }
                            is BoardListEntry.Card -> item(key = entry.key) {
                                val row = entry.row
                                val agents = if (speaks) boardAgents(row, fleet.worktrees) else emptyList()
                                TaskCardRow(
                                    row = row,
                                    agents = agents,
                                    orchestrator = boardOrchestrator(row, fleet.worktrees),
                                    speaks = speaks,
                                    presence = row.agentPresence(agents.size, speaks),
                                    onOpen = {
                                        connection.readsSync.open(workspace.id, row)
                                        onOpenTask(row.id)
                                    },
                                    onJump = jump,
                                )
                                Separator()
                            }
                        }
                    }
                    if (showsPlan) {
                        planItems(
                            state = planStates[workspace.id],
                            statuses = board.rows.associate { it.id to it.status },
                            onOpen = onOpenPlan,
                            onRetry = { scope.launch { connection.plans.read(workspace) } },
                            landedOpen = landedOpen,
                            onToggleLanded = { landedOpen = !landedOpen },
                        )
                    } else if (board.unreadable.isNotEmpty()) {
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
        SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter))
        if (askingMarkAll && summary != null && board != null) {
            MarkAllReadDialog(
                message = BoardSummary.markAllReadMessage(summary.taskCount, connection.readsSync.areShared(workspace.id)),
                onConfirm = {
                    askingMarkAll = false
                    connection.readsSync.markAllRead(workspace.id, board.rows, notes.values.flatten().maxOfOrNull { it.atMs })
                },
                onDismiss = { askingMarkAll = false },
            )
        }
    }
}

/**
 * A status's header: its title and count, and a chevron when it has tasks to
 * show or hide. An empty one says 0 and does nothing when tapped.
 */
@Composable
internal fun SectionHeader(header: BoardListEntry.Header, onToggle: () -> Unit) {
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
        Spacer(Modifier.weight(1f))
        // Trailing, quiet, in tabular digits, as the Mac's headers count (ov-104).
        Text(
            "${header.count}",
            style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
            color = MaterialTheme.colorScheme.outline,
        )
        Spacer(Modifier.width(8.dp))
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

/** "All Done  94 ›" under Done or Canceled: the History page (ov-103). */
@Composable
private fun HistoryRow(entry: BoardListEntry.History, onClick: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .heightIn(min = 48.dp)
            .padding(horizontal = 16.dp)
            .testTag("board-history-${entry.status.wire}")
            .semantics(mergeDescendants = true) { contentDescription = "${entry.title}, ${entry.total}" },
    ) {
        Text(entry.title, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.weight(1f))
        Text(
            "${entry.total}",
            style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
            color = MaterialTheme.colorScheme.outline,
        )
        Spacer(Modifier.width(8.dp))
        Icon(
            Icons.AutoMirrored.Outlined.KeyboardArrowRight,
            contentDescription = null,
            modifier = Modifier.size(18.dp),
            tint = MaterialTheme.colorScheme.outline,
        )
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

/** A board with no tasks: what it is, who fills it, and the shape of a card (ov-205, ov-245). */
@Composable
internal fun BoardBlank(led: Boolean, orchestratorRunning: Boolean, onShowOrchestrator: (() -> Unit)?) {
    val copy = PhoneFirstRun.blankCopy(led, orchestratorRunning)
    EmptyState(
        title = FirstRunCopy.BOARD_TITLE,
        detail = copy.lede,
        modifier = Modifier.fillMaxSize().testTag("board-empty"),
    ) {
        if (copy.rows.isNotEmpty()) PhoneEmptyRows(copy)
        // The skeleton shows the shape of what will arrive, so where the rows say
        // to start an orchestrator first, it only crowds them (ov-266).
        val offers = PhoneFirstRun.offersOrchestrator(led, orchestratorRunning) && onShowOrchestrator != null
        if (!offers) TaskSkeleton(Modifier.widthIn(max = 240.dp).padding(vertical = 8.dp))
        if (offers && onShowOrchestrator != null) {
            OutlinedButton(
                onClick = onShowOrchestrator,
                modifier = Modifier.testTag("board-show-orchestrator"),
            ) { Text(FirstRunCopy.SHOW_ORCHESTRATOR) }
        }
    }
}

@Composable
private fun Empty(title: String, detail: String) {
    EmptyState(title, detail, Modifier.fillMaxSize())
}

/** One card: key, title, what it asks, how long it has sat, acceptance, and its agent. */
@Composable
internal fun TaskCardRow(
    row: TaskRow,
    agents: List<Pair<Terminal, String>>,
    orchestrator: Pair<Terminal, String>?,
    speaks: Boolean,
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
        supportingContent = { CardDetails(row, now, speaks) },
        trailingContent = { AgentControl(row.key, agents, orchestrator, presence, onJump) },
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
private fun CardDetails(row: TaskRow, now: Long, speaks: Boolean = true) {
    val amber = glanceColor(GlancePalette.amber)
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        row.callToAction?.let {
            Text(it, color = amber, style = MaterialTheme.typography.labelMedium)
        }
        // What holds the card back, in amber because a block is the one thing
        // here that needs attention (ov-212), then when it starts and who is on
        // it, quiet. AgentKit's sentences, so the iPhone and the Mac say the
        // same ones.
        row.blockedSummary?.let {
            Text(
                it,
                color = amber,
                style = MaterialTheme.typography.labelMedium,
                modifier = Modifier.testTag("board-blocked-${row.key}"),
            )
        }
        row.startLine(now, speaks)?.let {
            Text(
                it,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.testTag("board-start-${row.key}"),
            )
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

/**
 * A chip for one agent, a chip with a menu for several, a "Subagent" chip into
 * the orchestrator's pane, a quiet "No agent", or nothing.
 */
@Composable
private fun AgentControl(
    key: String,
    agents: List<Pair<Terminal, String>>,
    orchestrator: Pair<Terminal, String>?,
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
        is TaskAgentPresence.Subagents ->
            // The orchestrator's pane, where the subagents live. Plain text
            // when that pane is not known: a chip that leads nowhere is worse.
            if (orchestrator != null) {
                AssistChip(
                    onClick = { onJump(orchestrator.first) },
                    label = { Text(presence.title.orEmpty()) },
                    trailingIcon = {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowForward,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                    },
                    modifier = Modifier.testTag("board-subagent-$key"),
                )
            } else {
                Text(
                    presence.title.orEmpty(),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.testTag("board-subagent-$key"),
                )
            }
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
    /** Another task's screen, from a key in this one's text (ov-196). */
    onNavigate: (Route) -> Unit,
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
    // Opened: read (ov-104), so its finish no longer keeps it in Done's short
    // list, and (ov-113) on every device when the runner keeps read state.
    LaunchedEffect(row?.id) {
        row?.let { connection.readsSync.open(workspaceId, it) }
    }
    // What this task asks of you, as the runner put it: a decision with its
    // options, a review, or an ask its agent holds. Answered here as on Needs
    // You, by the same row.
    // What its agents spent (ov-195), read as it opens.
    var usageReads by remember { mutableIntStateOf(0) }
    val usage by produceState<TaskUsageState>(TaskUsageState.Loading, taskId, usageReads) {
        value = TaskUsageState.Loading
        value = connection.taskUsage(taskId)
    }
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
        // Ask the orchestrator (ov-241): on while the workspace has one running.
        val seat = AskAboutTask.seat(workspaceId, fleet.worktrees, fleet.workspaces)
        val clipboard = LocalClipboard.current
        var askInFlight by remember { mutableStateOf(false) }
        var askNotice by remember(taskId) { mutableStateOf<String?>(null) }
        // "ov-190" in its text opens that task (ov-196).
        val taskKeys = rememberTaskKeyLinker(connection, onNavigate)
        CompositionLocalProvider(LocalTaskKeyLinker provides taskKeys) {
            LazyColumn(Modifier.fillMaxSize().padding(padding).testTag("board-detail")) {
                item(key = "heading") {
                    Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(row.title, style = MaterialTheme.typography.titleLarge)
                        Text(
                            row.status.title,
                            style = MaterialTheme.typography.labelLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        CardDetails(row.copy(acceptance = emptyList()), rememberMinuteClock(), speaks)
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
                            trailingContent = {
                                AgentControl(row.key, agents, boardOrchestrator(row, fleet.worktrees), presence, jump)
                            },
                        )
                    }
                }
                askOrchestratorItem(available = seat != null && !askInFlight, notice = askNotice) {
                    val orchestrator = seat ?: return@askOrchestratorItem
                    askInFlight = true
                    askNotice = null
                    scope.launch {
                        val delivery = AskAboutTask.deliver(
                            row.key, row.title, orchestrator.terminal.isAgentPane,
                            offer = { connection.composerHandoff.offer(orchestrator.terminal.id, it) },
                            paste = { connection.draftPrompt(orchestrator.terminal.id, it) },
                            copy = { clipboard.writeText("Far Cooler", it) },
                        )
                        askInFlight = false
                        if (delivery == AskAboutTask.Delivery.COPIED) {
                            // Stays here: the reference is on the clipboard and the
                            // notice says so, where a person pasting it can read it.
                            askNotice = AskAboutTask.copiedNotice(row.key)
                        } else {
                            onJump(TerminalRef(connection.host.id, orchestrator.worktreeId, orchestrator.terminal.id))
                        }
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
                            blockSpacing = 8.dp,
                            modifier = Modifier.padding(horizontal = 16.dp).testTag("board-detail-intent"),
                        )
                    }
                }
                row.acceptanceProgress?.let { progress ->
                    item(key = "acceptance-header") { Section("Acceptance · ${progress.sentence}") }
                    items(row.acceptance, key = { "line/${it.id}" }) { line ->
                        val linked = inline(line.text)
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
                                    linked,
                                    color = if (line.met) MaterialTheme.colorScheme.onSurfaceVariant
                                    else MaterialTheme.colorScheme.onSurface,
                                    textDecoration = if (line.met) TextDecoration.LineThrough else null,
                                )
                            },
                            modifier = Modifier
                                .testTag("board-acceptance-${line.id}")
                                .semantics {
                                    // What it says, not its markup.
                                    contentDescription = "${Markdown.plain(line.text)}. ${if (line.met) "Met" else "Not met"}"
                                    // Its task links, which the description
                                    // replaces: "Open ov-190" (ov-196).
                                    customActions = taskKeys.actions(linked)
                                },
                        )
                    }
                }
                taskUsageItems(usage, onRetry = { usageReads++ }) { Section("Usage") }
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
}

/**
 * "Answer" on a task in Needs Decision whose runner didn't list the
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
            OutlinedButton(onClick = { writing = true }) { Text("Answer") }
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
