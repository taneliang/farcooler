package com.farcooler.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
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
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.background
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.core.CoreException
import com.farcooler.data.Runner
import com.farcooler.model.AgentActivity
import com.farcooler.model.GlancePalette
import com.farcooler.model.NeedsYou
import com.farcooler.model.NeedsYouAnswer
import com.farcooler.model.NeedsYouButton
import com.farcooler.model.NeedsYouKind
import com.farcooler.model.NeedsYouRow
import com.farcooler.model.RunnerCount
import com.farcooler.model.reassurance
import com.farcooler.net.Connection
import com.farcooler.net.rethrowIfCancellation
import kotlinx.coroutines.launch

/**
 * What the phone opens onto: every item, on every runner, that needs a person,
 * then the workspaces (spec §6).
 *
 * The items are the rollup's (spec §2): held asks, blocked agents, tasks in
 * Needs Decision, tasks In Review — each runner's own list, merged by rank in
 * `model/NeedsYou.kt`, and each row labeled with its workspace. An ask and a
 * decision are answered here, in place, with the runner's own options; a
 * blocked agent and a review open. It used to be a section per worktree, with
 * finished agents and unread diffs, and Board rows beneath; ruling 1 took the
 * finished agents and diffs out of the inbox, and a decision is an item now.
 *
 * Under the items, the Workspaces list: each repository's workspaces, each
 * with its orchestrator's mark and its count, then its Unclaimed and Hidden
 * worktrees. It's the way into everything else, and the drawer holds the same
 * list.
 *
 * Every runner at once, as before: an ask on a runner in another room is as
 * urgent as one on this desk, and the order spans runners because a rank is a
 * duration, not a clock reading.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun NeedsYouScreen(model: AppModel, onOpenDrawer: () -> Unit) {
    val entries by model.fleet.entries.collectAsStateWithLifecycle()
    val connections by model.fleet.active.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()

    var refreshing by remember { mutableStateOf(false) }
    var editingRunner by remember { mutableStateOf<Runner?>(null) }

    val runners = rememberNeedsYouRunners(connections)
    val rows = NeedsYou.rows(runners.map { it.second })
    val merged = rows.map { it.entry }
    val namesRunners = connections.size > 1
    val older = NeedsYou.olderRunners(runners.map { it.second })
    val sections = runners.flatMap { (connection, runner) ->
        NeedsYou.workspaces(runner, merged).map { connection to it }
    }

    // What's running, for the sentence under "Nothing needs you": per runner,
    // because only an answering runner's count is believed. Hidden worktrees
    // are left out, as they always were here.
    val visible = entries.filter { !it.worktree.isHidden }
    val working = visible.groupBy { it.host.id }.mapValues { (_, entries) ->
        entries.sumOf { entry -> entry.worktree.terminals.count { it.agent == AgentActivity.WORKING } }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Needs You") },
                navigationIcon = {
                    IconButton(onClick = onOpenDrawer) {
                        Icon(Icons.Outlined.Menu, contentDescription = "Show workspaces")
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
                    connections.forEach { it.readNeedsYou() }
                    refreshing = false
                }
            },
            modifier = Modifier.padding(padding),
        ) {
            LazyColumn(Modifier.fillMaxSize().testTag("needs-you")) {
                // Above the items, because a runner nobody can reach is the
                // reason the items below may not be everything.
                items(connections, key = { "runner/${it.host.id}" }) { connection ->
                    RunnerStatusRow(
                        connection = connection,
                        showLabel = namesRunners,
                        onRetry = { model.fleet.retry(connection.host.id) },
                        onReconnectNow = { connection.reconnectNow() },
                        onTrust = { fingerprint ->
                            model.hosts.trust(connection.host, fingerprint)
                            model.fleet.retry(
                                connection.host.id,
                                connection.host.copy(fingerprint = fingerprint),
                            )
                        },
                        // Both halves: forgetting a key that was never pinned
                        // changes nothing, and the dial is what puts the
                        // fingerprint back on screen.
                        onReviewKey = {
                            model.hosts.forgetKey(connection.host)
                            model.fleet.retry(
                                connection.host.id,
                                connection.host.copy(fingerprint = null),
                            )
                        },
                        onNotNow = { connection.declineHostKey() },
                        onEdit = { editingRunner = connection.host },
                    )
                }

                val people = runners.map { it.second }
                if (NeedsYou.nothingNeedsYou(people, rows)) {
                    item(key = "reassurance") {
                        Reassurance(connections, working, visible.size, NeedsYou.caveat(NeedsYou.unanswered(people)))
                    }
                } else if (rows.isEmpty()) {
                    // No runner's list is read yet, so "Nothing needs you"
                    // would be a claim nobody made.
                    item(key = "checking") { Checking() }
                }

                items(rows, key = { "item/${it.key}" }) { row ->
                    val connection = connections.firstOrNull { it.host.id == row.hostId }
                    NeedsYouItemRow(
                        row = row,
                        connection = connection,
                        onOpen = { model.openItem(row.hostId, row.item) },
                        modifier = Modifier.animateItem(),
                    )
                }

                // An older runner can say only which agents are blocked; its
                // asks, decisions and reviews may be waiting anyway.
                items(older, key = { "older/$it" }) { runner ->
                    Text(
                        NeedsYou.olderRunnerNote(runner),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }

                if (sections.isNotEmpty()) {
                    item(key = "workspaces") {
                        Text(
                            "Workspaces",
                            style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.padding(start = 16.dp, top = 24.dp, bottom = 0.dp),
                        )
                    }
                }
                workspaceItems(
                    sections = sections,
                    namesRunners = namesRunners,
                    onOpenWorkspace = { model.openWorkspace(it.hostId, it.workspace.id) },
                    onOpenWorktrees = { host, repository, hidden ->
                        model.navigate(Route.Worktrees(host, repository, hidden))
                    },
                )
            }
        }
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
 * One item: its mark, where it is, what it asks, and its buttons.
 *
 * The row opens the item (its workspace, task and agent; see
 * `AppModel.openItem`). The buttons answer it in place. A button that has sent
 * shows a spinner where it was; a refused answer leaves one line under the
 * row, in the words of spec §2.5, and the item stays until the runner says it
 * has gone.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun NeedsYouItemRow(
    row: NeedsYouRow,
    connection: Connection?,
    onOpen: () -> Unit,
    modifier: Modifier = Modifier,
    /** False on the task screen, which is already the item's place. */
    showPlace: Boolean = true,
) {
    val scope = rememberCoroutineScope()
    val item = row.item
    val daemon = connection?.daemon?.collectAsStateWithLifecycle()?.value
    // Below Control scope a runner sends no actions, and "Answer…" would be
    // refused. Unknown is not "read": see `DaemonBuild.grantedScope`.
    val mayAnswer = daemon?.grantedScope != "read" && connection != null
    val buttons = NeedsYouAnswer.buttons(item, mayAnswer)
    var sending by remember(item.id) { mutableStateOf<String?>(null) }
    var refusal by remember(item.id) { mutableStateOf<String?>(null) }
    var writing by remember(item.id) { mutableStateOf(false) }
    var more by remember(item.id) { mutableStateOf(false) }
    val agent = item.terminal?.label?.ifBlank { null } ?: "the agent"

    fun send(id: String, answer: suspend (Connection) -> Unit) {
        val live = connection ?: return
        sending = id
        refusal = null
        scope.launch {
            try {
                answer(live)
            } catch (e: Exception) {
                e.rethrowIfCancellation()
                refusal = NeedsYouAnswer.refusal((e as? CoreException)?.what, agent)
            } finally {
                sending = null
            }
        }
    }

    fun answer(id: String, body: String) {
        when (item.kindValue) {
            NeedsYouKind.ASK -> {
                val terminal = item.terminal?.id ?: return
                val ask = item.askId ?: return
                send(id) { it.answerAsk(terminal, ask, id) }
            }
            NeedsYouKind.DECISION -> {
                val task = item.task?.id ?: return
                send(id) { it.answerDecision(task, body) }
            }
            else -> onOpen()
        }
    }

    Column(
        modifier
            .fillMaxWidth()
            .clickable(onClick = onOpen)
            .padding(horizontal = 16.dp, vertical = 10.dp)
            .testTag("needs-you-item-${item.id}"),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            KindMark(item.kindValue)
            Spacer(Modifier.width(8.dp))
            Text(
                if (showPlace) listOfNotNull(row.place, row.runner).joinToString(" · ") else kindTitle(item.kindValue),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Text(
            item.question.ifBlank { kindTitle(item.kindValue) },
            style = MaterialTheme.typography.bodyLarge,
            maxLines = 3,
            overflow = TextOverflow.Ellipsis,
        )
        subject(row).takeIf { showPlace }?.let {
            Text(
                it,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        item.detail?.takeIf { it.isNotBlank() }?.let {
            Text(
                it,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for (button in buttons) {
                when (button) {
                    is NeedsYouButton.Answer -> {
                        val action = button.action
                        if (sending == action.id) {
                            Box(Modifier.size(40.dp), contentAlignment = Alignment.Center) {
                                CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                            }
                        } else if (action.primary) {
                            Button(onClick = { answer(action.id, action.id) }, enabled = sending == null) {
                                Text(action.title.ifBlank { action.id })
                            }
                        } else {
                            OutlinedButton(onClick = { answer(action.id, action.id) }, enabled = sending == null) {
                                Text(
                                    action.title.ifBlank { action.id },
                                    color = if (action.destructive) MaterialTheme.colorScheme.error
                                    else MaterialTheme.colorScheme.primary,
                                )
                            }
                        }
                    }
                    is NeedsYouButton.More -> Box {
                        OutlinedButton(onClick = { more = true }, enabled = sending == null) { Text("More") }
                        DropdownMenu(expanded = more, onDismissRequest = { more = false }) {
                            button.actions.forEach { action ->
                                DropdownMenuItem(
                                    text = { Text(action.title.ifBlank { action.id }) },
                                    onClick = {
                                        more = false
                                        answer(action.id, action.id)
                                    },
                                )
                            }
                        }
                    }
                    NeedsYouButton.Write ->
                        if (sending == WRITTEN) {
                            Box(Modifier.size(40.dp), contentAlignment = Alignment.Center) {
                                CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                            }
                        } else {
                            OutlinedButton(onClick = { writing = true }, enabled = sending == null) {
                                Text("Answer…")
                            }
                        }
                    is NeedsYouButton.Open -> TextButton(onClick = onOpen) { Text(button.title) }
                }
            }
        }
        refusal?.let {
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
        }
    }

    if (writing) {
        var text by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { writing = false },
            title = { Text(item.task?.key?.let { "Answer $it" } ?: "Answer") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(item.question, style = MaterialTheme.typography.bodyMedium)
                    OutlinedTextField(value = text, onValueChange = { text = it }, minLines = 2)
                }
            },
            confirmButton = {
                TextButton(
                    enabled = text.isNotBlank(),
                    onClick = {
                        writing = false
                        answer(WRITTEN, text.trim())
                    },
                ) { Text("Send") }
            },
            dismissButton = { TextButton(onClick = { writing = false }) { Text("Cancel") } },
        )
    }
}

/** What a typed answer's spinner is keyed by: no option has this id. */
private const val WRITTEN = "\u0000written"

/** The task an item is about, "bil-7 Invoice PDF export", or its pane and worktree. */
private fun subject(row: NeedsYouRow): String? {
    val item = row.item
    item.task?.let { task -> return listOf(task.key, task.title).filter { it.isNotBlank() }.joinToString(" ") }
    val terminal = item.terminal ?: return null
    val who = if (terminal.role == "orchestrator") "Orchestrator" else terminal.label
    val where = item.worktree?.name?.takeIf { it.isNotBlank() }
    return listOfNotNull(who.ifBlank { null }, where).joinToString(" in ").ifBlank { null }
}

private fun kindTitle(kind: NeedsYouKind): String = when (kind) {
    NeedsYouKind.ASK -> "Asking to use a tool"
    NeedsYouKind.BLOCKED -> "Needs You"
    NeedsYouKind.DECISION -> "Needs a decision"
    NeedsYouKind.REVIEW -> "Ready for review"
    NeedsYouKind.UNKNOWN -> "Needs You"
}

/** Amber for what waits on you, the review ink for a review. */
@Composable
private fun KindMark(kind: NeedsYouKind) {
    val color = glanceColor(if (kind == NeedsYouKind.REVIEW) GlancePalette.review else GlancePalette.amber)
    Box(Modifier.size(PROCESS_DOT).clip(CircleShape).background(color))
}

/**
 * The most common state, and it is doing a job rather than filling a gap.
 *
 * "Is anything wrong?" is the question this app is opened with most often, and
 * the honest answer is usually no. A blank screen answers it too, and answers
 * it badly: an empty list is indistinguishable from a list that has not loaded,
 * from a runner that stopped talking, and from a bug.
 *
 * **The caveat under it is the part iOS has no need for.** "Nothing needs you"
 * is an assertion about the whole fleet, and this app's fleet is every runner
 * at once — so a runner that is failed, reconnecting or still shaking hands
 * makes the sentence a claim the app is not entitled to. iOS never had to say
 * this because its inbox speaks for the one connection it is attached to and
 * says so in its own subtitle. Here the count is unqualified because it really
 * is everything; the price of that is owning up when it is not.
 *
 * The caveat is [NeedsYou.caveat], iOS's words, naming the runners that aren't
 * answering or whose list wasn't read: a connected runner whose read failed is
 * as unknown as one that's down.
 *
 * Said only in this block, and not over a list that has rows in it. When there
 * are rows, the runner rows at the top of the screen are already saying which
 * runner is quiet and offering the one useful thing to do about it; repeating
 * it under a list would be the same news twice.
 */
@Composable
private fun Reassurance(
    connections: List<Connection>,
    working: Map<String, Int>,
    worktrees: Int,
    caveat: String?,
) {
    val runners = connections.map { connection ->
        key(connection.host.id) {
            val link by connection.link.collectAsStateWithLifecycle()
            val fleet by connection.fleet.collectAsStateWithLifecycle()
            // Healthy too: without it the front door could never say tmux is
            // down, and said "Nothing is running" instead.
            RunnerCount(link, working[connection.host.id] ?: 0, fleet.runtimeHealthy)
        }
    }
    val where = if (connections.size == 1) " on ${connections[0].host.displayLabel}" else ""

    EmptyState(
        title = "Nothing needs you",
        detail = reassurance(runners, where, worktrees),
        icon = Icons.Outlined.CheckCircleOutline,
    ) {
        if (caveat != null) {
            Text(
                caveat,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.testTag("needs-you-caveat"),
            )
        }
    }
}

/** Before any runner's list is read: not "Nothing needs you", which nobody said. */
@Composable
private fun Checking() {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 40.dp).testTag("needs-you-checking"),
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
        Text("Checking what needs you…", color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

