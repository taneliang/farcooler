package com.farcooler.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.Difference
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
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
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.data.Runner
import com.farcooler.model.AGENTS_PER_WORKSPACE
import com.farcooler.model.AgentActivity
import com.farcooler.model.NeedsYouSection
import com.farcooler.model.blockedOverflow
import com.farcooler.model.finishedOverflow
import com.farcooler.model.needsYou
import com.farcooler.net.Connection
import com.farcooler.net.TerminalRef
import kotlinx.coroutines.launch

/**
 * What the phone opens onto: everything, on every runner, that wants a person.
 *
 * The app used to open into a terminal. `FleetRepository.landing()` picked one
 * on connect and the workspace list was the fallback for a fleet with nothing
 * running — the right front door for exactly one of the four situations
 * `docs/jobs-to-be-done.md` names, on the couch about to drive an agent. In the
 * other three, the first question is *what needs me*, and a terminal is an
 * answer to a question nobody asked. iOS deleted the same shape in `1be6264`.
 *
 * ## One section per workspace, spanning every runner
 *
 * The derivation and its ordering live in `model/NeedsYou.kt`, where they can
 * be tested without a device; the argument for grouping by workspace rather
 * than by RUNNER is written down there too, and it is the one decision on this
 * screen that is not a port. In short: grouping by runner is iOS's
 * `HostSwitcherBar` in list form, and it would let the most urgent thing in the
 * fleet sit halfway down the screen under a heading for a machine with nothing
 * to say.
 *
 * The runner is on every section instead — [NeedsYouSection.hostId] in the key
 * and the tap, its name on the header's second line, and only once more than
 * one runner is connected. That last rule is `FleetScreen`'s already: with a
 * single runner its name is on every row and says nothing about which row is
 * which.
 *
 * ## This screen sorts by rank, and the fleet list still does not
 *
 * `231f81a` decoded `sortRank` and deliberately did NOT sort the fleet list by
 * it, because a row that slides as an agent finishes takes the tap you had
 * already committed to. That reasoning is right and it does not transfer,
 * because the two lists have different jobs:
 *
 * - **The fleet list is a map.** It holds every pane on every runner, wanted or
 *   not, and you navigate it by memory — the row you tapped yesterday is where
 *   you left it. Attention is a MARK on a row there, and a mark you can find in
 *   a list that holds still beats one that comes to you by moving the list.
 * - **The front door is a queue.** Every row on it is a row you already care
 *   about, there are typically none to five of them, and the answer to "what
 *   needs me first" IS an ordering. A queue in creation order is not a queue.
 *
 * The slide-under-the-thumb hazard is paid for rather than waved away, three
 * ways. `feed::rank` is tiered a whole `TIER_SPAN` apart and ordered by AGE
 * inside a tier, so relative order only changes when an agent actually crosses
 * a tier boundary — never as a working pane merely ages, which is most of what
 * moves in the fleet list. The tiebreak in [needsYou] is total, so equal ranks
 * cannot swap on a poll. And every row here carries `Modifier.animateItem()`,
 * so the motion that is left is motion you can see happening rather than a row
 * teleporting mid-reach.
 *
 * ## Material, not HIG
 *
 * `LazyColumn` with `stickyHeader`, so the worktree a row belongs to stays on
 * screen while you read down its agents — which is the thing iOS's inset cards
 * were doing structurally and which this does not reproduce. Sentence case
 * throughout, settled in `cb13d31`.
 *
 * `ListItem` for the two rows that fit it — the diff and the way to the
 * workspace list. NOT for an agent: [TerminalRow] is four bands running one to
 * eight lines, and `ListItem` has three text slots and a specified minimum
 * height per variant. Drawing an agent a second way here would also be a second
 * chance for two screens to say different things about one pane.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun NeedsYouScreen(
    model: AppModel,
    onSelect: (TerminalRef) -> Unit,
    onReviewChanges: (hostId: String, workspaceId: String) -> Unit,
    onOpenWorkspaces: () -> Unit,
    onOpenDrawer: () -> Unit,
) {
    val entries by model.fleet.entries.collectAsStateWithLifecycle()
    val connections by model.fleet.active.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()

    var refreshing by remember { mutableStateOf(false) }
    var editingRunner by remember { mutableStateOf<Runner?>(null) }

    // Derived from the entries alone, which already carry the counts — see
    // `FleetEntry.counts`. Nothing here re-subscribes per runner.
    val sections = remember(entries) { needsYou(entries.map { it.needsYouInput() }) }
    val visible = entries.filter { !it.workspace.isHidden }
    val namesRunners = connections.size > 1
    // Derived from `entries`, which is what this composable is subscribed to.
    // Reading it off each connection's fleet would be a value nothing here
    // observes, so the sentence under "Nothing needs you" would go stale the
    // moment the last agent finished.
    val working = visible.sumOf { entry ->
        entry.workspace.terminals.count { it.agent == AgentActivity.WORKING }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Needs you") },
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
            LazyColumn(Modifier.fillMaxSize()) {
                // Above the sections, because a runner nobody can reach is the
                // reason the sections below may not be everything. Each row
                // draws nothing at all while its runner is answering.
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
                        onReviewKey = {
                            model.hosts.forgetKey(connection.host)
                            model.fleet.retry(
                                connection.host.id,
                                connection.host.copy(fingerprint = null),
                            )
                        },
                        onEdit = { editingRunner = connection.host },
                    )
                }

                if (sections.isEmpty()) {
                    item(key = "reassurance") {
                        Reassurance(connections, working, visible.size)
                    }
                }

                for (section in sections) {
                    stickyHeader(key = "header/${section.key}") {
                        SectionHeader(section, showRunner = namesRunners)
                    }

                    items(
                        section.blocked.take(AGENTS_PER_WORKSPACE),
                        key = { "agent/${section.hostId}/${it.id}" },
                    ) { terminal ->
                        AgentRow(model, section, terminal, onSelect, Modifier.animateItem())
                    }
                    if (section.blocked.size > AGENTS_PER_WORKSPACE) {
                        item(key = "more-blocked/${section.key}") {
                            Overflow(blockedOverflow(section.blocked.size - AGENTS_PER_WORKSPACE))
                        }
                    }

                    // Below the blocked ones and above the diff. The same row,
                    // deliberately: `TerminalRow` already says "Done" with a
                    // green check and "Failed" with a red cross, so a finished
                    // agent is drawn here exactly as it is drawn in the fleet
                    // list and the tab strip.
                    //
                    // Tapping one is what ENDS it — `terminal.seen` clears
                    // `done` to `idle` on the next poll and the row goes. That
                    // is the intended shape of this row, not a wrinkle in it.
                    items(
                        section.finished.take(AGENTS_PER_WORKSPACE),
                        key = { "agent/${section.hostId}/${it.id}" },
                    ) { terminal ->
                        AgentRow(model, section, terminal, onSelect, Modifier.animateItem())
                    }
                    if (section.finished.size > AGENTS_PER_WORKSPACE) {
                        item(key = "more-finished/${section.key}") {
                            Overflow(
                                finishedOverflow(section.finished.drop(AGENTS_PER_WORKSPACE))
                            )
                        }
                    }

                    if (section.showsChanges) {
                        item(key = "changes/${section.key}") {
                            ChangesRow(section, onReviewChanges, Modifier.animateItem())
                        }
                    }
                }

                item(key = "workspaces") {
                    WorkspacesRow(visible.size, entries.size, connections, onOpenWorkspaces)
                }
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
 * One agent, wherever it came from.
 *
 * Shared by the blocked group and the finished one because the row is the same
 * row and the destination is the same destination: the workspace, opened on
 * that pane, which is what somebody wants from both — the answer to the
 * question, or the answer to the question they asked.
 *
 * `point`, not `choose`. [AppModel.open] is the sent-here door and does not
 * write the remembered tab down: arriving from the front door is the app
 * routing you, not a preference about where this workspace opens tomorrow. The
 * tab strip stays the one writer. See [Focus].
 */
@Composable
private fun AgentRow(
    model: AppModel,
    section: NeedsYouSection,
    terminal: com.farcooler.model.Terminal,
    onSelect: (TerminalRef) -> Unit,
    modifier: Modifier = Modifier,
) {
    val scope = rememberCoroutineScope()
    // Looked up rather than carried on the section: a reconnect replaces the
    // Connection, and a row holding the old one would act on a dead session.
    val connection = model.fleet.connection(section.hostId)
    Column(modifier) {
        TerminalRow(
            terminal = terminal,
            ordinal = section.ordinals[terminal.id],
            onClick = {
                onSelect(TerminalRef(section.hostId, section.workspace.id, terminal.id))
            },
            onAction = { action ->
                connection?.let { scope.launch { it.act(action, terminal) } }
            },
        )
    }
}

/**
 * The workspace's own line: what the work is, which branch it is on, and — once
 * there is more than one — whose runner.
 *
 * A header, not a target, so nothing here competes with the rows below it and
 * they read as things inside this worktree without an indent having to say so.
 * Sticky, so the worktree stays named while you read down its agents.
 *
 * Deliberately no amber up here, and no counts. That colour is reserved across
 * this app for an agent waiting on you and [TerminalRow] already spends it on
 * exactly those, inside; a second mark here would say the same thing twice and
 * weaken it both times. The counts moved down to the row that opens them.
 *
 * An opaque background, because a sticky header scrolls OVER the rows behind it
 * — the one thing this needs that the fleet list's version of the same header
 * does not.
 */
@Composable
private fun SectionHeader(section: NeedsYouSection, showRunner: Boolean) {
    Column(
        Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface)
            .padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 2.dp)
    ) {
        Text(
            section.workspace.task.ifBlank { section.workspace.branch },
            style = MaterialTheme.typography.titleSmall,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            buildString {
                // Which branch is not the useful fact about the main checkout;
                // that it IS the repository is. `FleetScreen`'s header and
                // iOS's both say it the same way.
                append(
                    if (section.workspace.isMainCheckout) "Primary checkout"
                    else section.workspace.branch
                )
                if (showRunner) append(" · ${section.hostLabel}")
            },
            style = MaterialTheme.typography.labelSmall,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/**
 * The diff, as a row of its own — and it opens the diff.
 *
 * Why it gets a row at all: no road into a workspace had ever opened it on its
 * diff, so from the front door the diff cost two taps while `+82 -13` sat on
 * the header looking like the control for it.
 * `docs/jobs-to-be-done.md` F4 has the phone's review experience load-bearing
 * rather than a scaled-down Mac feature, which makes the diff the most
 * important target on this screen.
 *
 * **Both of the reasons this row used to point at the worktree instead are
 * spent, and the second one is worth keeping rather than deleting.** The first
 * was that the workspace screen drew a `changes` pane as raw VT bytes, so
 * aiming here at one would be worse than aiming at the worktree; the Changes
 * tab and [Pane]'s fold closed that. The second was the one that actually
 * decided it: that tab could say how big the diff was and could not show it,
 * and sending somebody who asked to review changes to a screen saying "not yet"
 * is worse than sending them to the agents that made them. There is a review
 * behind that tab now, and — as this comment predicted — the one line that had
 * to change is the tap.
 *
 * What it does NOT do is name a pane on the runner. The tab is asked for by
 * workspace id, so it answers during a handshake where a remembered terminal
 * cannot: tapping this row on a runner that is still connecting lands on the
 * diff and waits there, rather than on "Waiting for that runner." See
 * [AppModel.openChanges] for why the focus is written before the route moves.
 *
 * A `ListItem`, unlike the agent rows above it. This one is a leading icon,
 * one line of text and a trailing pair of numbers, which is exactly the shape
 * `ListItem` specifies — and giving it the same full width as the rows above
 * is what keeps every target in a section the same kind of thing. A small
 * target beside a large one, tapped while walking, is a coin toss with a wrong
 * side.
 */
@Composable
private fun ChangesRow(
    section: NeedsYouSection,
    onReview: (hostId: String, workspaceId: String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val counts = section.counts
    ListItem(
        headlineContent = { Text("Review changes") },
        leadingContent = {
            Icon(
                Icons.Outlined.Difference,
                contentDescription = null,
                modifier = Modifier.size(20.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        trailingContent = {
            // The counts, drawn by the one composable that knows what those
            // two colours mean — the workspace's Changes chip shows the same
            // pair for the same worktree, and they must not be able to come out
            // different. See [DiffCounts].
            if (counts != null && counts.hasDiff) DiffCounts(counts)
        },
        modifier = modifier
            .clickable { onReview(section.hostId, section.workspace.id) }
            // Spoken as one target, in the Changes chip's own words. `+82` and
            // `-13` read aloud as two orphaned numbers, and the clause about
            // uncommitted work exists nowhere else — so the row and the tab it
            // now opens say the same sentence about the same worktree, which
            // they could not while the row went somewhere else. Set on the
            // whole `ListItem` rather than on the counts, because a row with
            // two elements in it is a row TalkBack stops on twice.
            .semantics(mergeDescendants = true) {
                contentDescription = changesDescription(counts, lead = "Review changes")
            },
    )
}

/**
 * The line a group puts under itself when it ran out of room.
 *
 * Indented to the left edge of the words in the rows above it rather than to
 * the row's own edge, so the sentence summarizing a group sits inside the group.
 * `TerminalRow` starts its text 34dp in: a 16dp margin, an 8dp `PROCESS_DOT`
 * and a 10dp gap. Read as three numbers rather than one, so that changing any
 * of them there and not here is a visible mistake rather than a silent one.
 */
@Composable
private fun Overflow(sentence: String) {
    Text(
        sentence,
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
        modifier = Modifier.padding(start = 34.dp, top = 2.dp, bottom = 6.dp),
    )
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
 * Said only in this block, and not over a list that has rows in it. When there
 * are rows, the runner rows at the top of the screen are already saying which
 * runner is quiet and offering the one useful thing to do about it; repeating
 * it under a list would be the same news twice.
 */
@Composable
private fun Reassurance(connections: List<Connection>, working: Int, workspaces: Int) {
    val phases = connections.map { connection ->
        key(connection.host.id) { connection.phase.collectAsStateWithLifecycle().value }
    }
    val silent = phases.count { it !is Connection.Phase.Connected }

    Column(
        Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 40.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Icon(
            Icons.Outlined.CheckCircleOutline,
            contentDescription = null,
            modifier = Modifier.size(34.dp),
            tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
        )
        Text("Nothing needs you", style = MaterialTheme.typography.titleMedium)
        Text(
            reassuranceDetail(working, connections, workspaces),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (silent > 0) {
            Text(
                if (silent == 1) "One runner hasn’t answered, so this isn’t the whole fleet."
                else "$silent runners haven’t answered, so this isn’t the whole fleet.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/**
 * What is happening, given that nothing needs you.
 *
 * Counts `working` only, which is disjoint from the two states that put a row
 * on this screen — so this is the answer to "is anything happening", asked only
 * once nothing needs you. Hidden workspaces are left out, to agree with the
 * rest of the screen.
 *
 * Named runner only when there is exactly one connected. With one, naming it is
 * what tells you which machine the app is speaking for; with several the scope
 * is the whole fleet, and picking one of their names to put in the sentence
 * would be the runner picker sneaking back in through the copy.
 */
private fun reassuranceDetail(
    working: Int,
    connections: List<Connection>,
    workspaces: Int,
): String {
    val where = if (connections.size == 1) " on ${connections[0].host.displayLabel}" else ""
    if (working == 0 && workspaces == 0 && connections.isNotEmpty()) {
        return "Nothing is running$where yet."
    }
    return when (working) {
        0 -> "Nothing is running$where."
        1 -> "One agent is working$where."
        else -> "$working agents are working$where."
    }
}

/**
 * The door to the whole fleet, counting the thing that is actually behind it.
 *
 * It counts what the destination LISTS. iOS's version of this row said
 * "Working" over a count of agents mid-turn, which at 3am with nothing running
 * read `Working 0` on the only way in — a label telling you not to open the one
 * door you needed.
 *
 * Hidden workspaces are left out of the number, so it is a number you can find
 * by counting rows over there — and the supporting line says so when there are
 * any, because "12" over a list with fourteen rows in it is the same kind of
 * lie.
 *
 * A count of zero is still worth a tap: the quick task and the new-workspace
 * form live in that screen and nowhere else on the phone, so an empty fleet is
 * the state in which going there matters most.
 */
@Composable
private fun WorkspacesRow(
    visible: Int,
    total: Int,
    connections: List<Connection>,
    onOpen: () -> Unit,
) {
    val hidden = total - visible
    ListItem(
        headlineContent = { Text("Workspaces") },
        leadingContent = {
            Icon(
                Icons.Outlined.Folder,
                contentDescription = null,
                modifier = Modifier.size(20.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        supportingContent = {
            val sentence = when {
                hidden == 1 -> "1 more is hidden."
                hidden > 1 -> "$hidden more are hidden."
                connections.isEmpty() -> "No runners yet. This is where you add one."
                total == 0 -> "No workspaces yet. This is where you start one."
                else -> null
            }
            if (sentence != null) Text(sentence)
        },
        trailingContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("$visible", style = MaterialTheme.typography.labelLarge)
                Spacer(Modifier.width(4.dp))
            }
        },
        modifier = Modifier.clickable(onClick = onOpen),
    )
}


