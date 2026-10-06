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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.AccountTree
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.Circle
import androidx.compose.material.icons.outlined.FilterList
import androidx.compose.material.icons.outlined.Flag
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.HourglassEmpty
import androidx.compose.material.icons.outlined.Inbox
import androidx.compose.material.icons.outlined.Inventory2
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material.icons.outlined.Map
import androidx.compose.material.icons.outlined.PanTool
import androidx.compose.material.icons.outlined.PauseCircle
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material.icons.outlined.Sync
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.AutoAwesome
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.rememberModalBottomSheetState
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.model.Capability
import com.farcooler.model.OneTree
import com.farcooler.model.PlanPage
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanStrip
import com.farcooler.model.Plan
import com.farcooler.model.Terminal
import com.farcooler.model.WorkspaceSummary
import com.farcooler.model.GlancePalette
import com.farcooler.net.Connection
import kotlinx.coroutines.launch

// The plan's strip on the orchestrator and the bottom sheet it opens, and the
// One tree as the Themes tab and its pushed levels (ov-300). The rules are
// `PlanStrip` and `OneTree` in com.farcooler.model, ports of AgentKit's; this
// draws them in Material's idiom: a tonal pill for the strip, a modal bottom
// sheet that opens partly expanded (the peek), and list items for the tree.
// See .claude/agent/reports/phones-tree/design.md.

/** Where a tree row or a plan row sends the screen. */
class TreeNavigation(
    val onOpenTask: (String) -> Unit,
    val onOpenPlan: (PlanPage) -> Unit,
    /** A pane, by its worktree and terminal. */
    val onOpenTerminal: (worktree: String, terminal: String) -> Unit,
    /** A worktree opened whole. */
    val onOpenWorktree: (String) -> Unit,
    val onOpenLevel: (String) -> Unit,
)

/** The strip, under the tab row while the orchestrator is up, and the sheet it opens. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PlanStripBar(connection: Connection, workspace: WorkspaceSummary, orchestrator: Terminal?, onOpenPlan: (PlanPage) -> Unit) {
    val planStates by connection.plans.states.collectAsStateWithLifecycle()
    val boards by connection.boards.collectAsStateWithLifecycle()
    val list by connection.needsYou.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val keepsPlan = daemon?.can(Capability.BOARD_PLAN) == true
    val scope = rememberCoroutineScope()
    LaunchedEffect(workspace.id, keepsPlan) {
        if (boards[workspace.id] == null) connection.readBoard(workspace)
        if (keepsPlan && planStates[workspace.id] == null) connection.plans.read(workspace)
    }
    val plan = (planStates[workspace.id] as? PlanReadState.Loaded)?.plan ?: Plan()
    val strip = PlanStrip.of(plan, PlanStrip.needsYouCount(workspace, boards[workspace.id], plan, list), orchestrator)
    var peeking by remember { mutableStateOf(false) }
    if (!strip.isEmpty) PlanStripPill(strip) { peeking = true }
    if (peeking) {
        val state = rememberModalBottomSheetState(skipPartiallyExpanded = false)
        ModalBottomSheet(onDismissRequest = { peeking = false }, sheetState = state, modifier = Modifier.testTag("plan-sheet")) {
            LazyColumn(Modifier.fillMaxWidth()) {
                item(key = "orchestrator") { SheetHeader(strip) }
                if (keepsPlan) {
                    planItems(
                        state = planStates[workspace.id],
                        statuses = boards[workspace.id]?.rows.orEmpty().associate { it.id to it.status },
                        onOpen = { page ->
                            scope.launch {
                                state.hide()
                                peeking = false
                                onOpenPlan(page)
                            }
                        },
                        onRetry = { scope.launch { connection.plans.read(workspace) } },
                    )
                } else {
                    item(key = "needs-update") { PlanNotice(com.farcooler.model.PlanWords.NEEDS_UPDATE, null) }
                }
            }
        }
    }
}

/** The strip itself: the state's mark, the words, and the chevron that says it opens. */
@Composable
internal fun PlanStripPill(strip: PlanStrip, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        shape = CircleShape,
        tonalElevation = 3.dp,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag("plan-strip")
            .semantics(mergeDescendants = true) { contentDescription = strip.accessibilityLabel },
    ) {
        Row(Modifier.padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(stateIcon(strip.orchestrator), null, Modifier.size(18.dp), tint = tone(strip.orchestrator.tone))
            Spacer(Modifier.width(10.dp))
            StripWords(strip, Modifier.weight(1f))
            Icon(Icons.Outlined.KeyboardArrowUp, null, tint = MaterialTheme.colorScheme.outline)
        }
    }
}

@Composable
private fun StripWords(strip: PlanStrip, modifier: Modifier) {
    val amber = glanceColor(GlancePalette.amber)
    val text = androidx.compose.ui.text.buildAnnotatedString {
        var rest = strip.parts
        strip.needsYouWords?.let { needs ->
            pushStyle(androidx.compose.ui.text.SpanStyle(color = amber, fontWeight = FontWeight.SemiBold))
            append(needs)
            pop()
            rest = rest.drop(1)
            if (rest.isNotEmpty()) append(" · ")
        }
        append(rest.joinToString(" · "))
    }
    Text(text, style = MaterialTheme.typography.bodyMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = modifier)
}

@Composable
internal fun SheetHeader(strip: PlanStrip) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 8.dp).testTag("plan-sheet-orchestrator"), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(stateIcon(strip.orchestrator), null, Modifier.size(20.dp), tint = tone(strip.orchestrator.tone))
            Spacer(Modifier.width(10.dp))
            Text("Orchestrator · ${strip.orchestrator.word}", style = MaterialTheme.typography.titleMedium)
        }
        strip.line?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 3, overflow = TextOverflow.Ellipsis) }
        strip.needsYouWords?.let {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.testTag("plan-sheet-needs-you")) {
                Icon(Icons.Outlined.Flag, null, Modifier.size(18.dp), tint = glanceColor(GlancePalette.amber))
                Spacer(Modifier.width(8.dp))
                Text(it, color = glanceColor(GlancePalette.amber))
            }
        }
        HorizontalDivider(Modifier.padding(top = 8.dp))
    }
}

private fun stateIcon(state: PlanStrip.Orchestrator): ImageVector = when (state) {
    PlanStrip.Orchestrator.NONE -> Icons.Outlined.Person
    PlanStrip.Orchestrator.STARTING -> Icons.Outlined.HourglassEmpty
    PlanStrip.Orchestrator.WORKING -> Icons.Outlined.Sync
    PlanStrip.Orchestrator.NEEDS_YOU -> Icons.Outlined.PanTool
    PlanStrip.Orchestrator.FAILED, PlanStrip.Orchestrator.STOPPED -> Icons.Outlined.Warning
    PlanStrip.Orchestrator.DONE -> Icons.Outlined.CheckCircleOutline
    PlanStrip.Orchestrator.IDLE -> Icons.Outlined.PauseCircle
}

@Composable
private fun tone(tone: PlanStrip.Tone): Color = when (tone) {
    PlanStrip.Tone.ATTENTION -> glanceColor(GlancePalette.amber)
    PlanStrip.Tone.FAILURE -> MaterialTheme.colorScheme.error
    PlanStrip.Tone.QUIET -> MaterialTheme.colorScheme.onSurfaceVariant
}

// ---- the tree ----

private const val FILTER_PREFS = "farcooler.tree"

private fun filterKey(hostId: String, workspaceId: String) = "tree.filter.$hostId.$workspaceId"

/** [workspace]'s tree as [connection] holds it, under [filter]. */
@Composable
private fun rememberTree(connection: Connection, workspace: WorkspaceSummary, filter: OneTree.Filter): OneTree.Tree? {
    val planStates by connection.plans.states.collectAsStateWithLifecycle()
    val boards by connection.boards.collectAsStateWithLifecycle()
    val list by connection.needsYou.collectAsStateWithLifecycle()
    val fleet by connection.fleet.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val keepsPlan = daemon?.can(Capability.BOARD_PLAN) == true
    LaunchedEffect(workspace.id, keepsPlan) {
        if (boards[workspace.id] == null) connection.readBoard(workspace)
        if (keepsPlan && planStates[workspace.id] == null) connection.plans.read(workspace)
    }
    val board = boards[workspace.id] ?: return null
    val plan = (planStates[workspace.id] as? PlanReadState.Loaded)?.plan ?: Plan()
    return remember(board, plan, fleet, list, filter) {
        OneTree.build(workspace, board, plan, fleet.worktrees, list?.items.orEmpty(), filter)
    }
}

/** The Themes tab: the tree's root. */
@Composable
fun ThemesTab(model: AppModel, connection: Connection, workspace: WorkspaceSummary, nav: TreeNavigation) {
    val prefs = LocalContext.current.getSharedPreferences(FILTER_PREFS, android.content.Context.MODE_PRIVATE)
    val key = filterKey(connection.host.id, workspace.id)
    var filter by remember(key) { mutableStateOf(OneTree.Filter.parse(prefs.getString(key, null))) }
    val tree = rememberTree(connection, workspace, filter)
    var choosing by remember { mutableStateOf(false) }
    var describing by remember { mutableStateOf(false) }
    var naming by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("Showing: ${filter.title.lowercase()}", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
            Box {
                IconButton(onClick = { choosing = true }, modifier = Modifier.testTag("tree-filter")) {
                    Icon(Icons.Outlined.FilterList, contentDescription = "Show")
                }
                DropdownMenu(expanded = choosing, onDismissRequest = { choosing = false }) {
                    OneTree.Filter.entries.forEach { f ->
                        DropdownMenuItem(text = { Text(f.title) }, onClick = {
                            choosing = false
                            filter = f
                            prefs.edit().putString(key, f.name).apply()
                        })
                    }
                }
            }
        }
        when {
            tree == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator(Modifier.testTag("tree-loading")) }
            else -> LazyColumn(Modifier.fillMaxSize().testTag("tree-root")) {
                if (tree.work.isEmpty()) item(key = "empty") {
                    Text(
                        if (filter == OneTree.Filter.IN_REVIEW) "Nothing is in review." else "No cards yet. The orchestrator files them as it plans the work.",
                        color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(16.dp).testTag("tree-empty"),
                    )
                }
                items(tree.work, key = { it.id }) { TreeRow(it, nav) }
                if (tree.below.isNotEmpty()) item(key = "divider") { HorizontalDivider(Modifier.padding(vertical = 8.dp)) }
                items(tree.below, key = { it.id }) { TreeRow(it, nav) }
                // The Worktrees tab's two ways to make one, kept: each claims it
                // for this workspace.
                item(key = "new") {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                        androidx.compose.material3.TextButton(onClick = { describing = true }, modifier = Modifier.testTag("new-worktree")) { Text("New worktree") }
                        androidx.compose.material3.TextButton(onClick = { naming = true }) { Text("Name a new worktree") }
                    }
                }
            }
        }
    }
    if (describing) QuickTaskSheet(model = model, workspace = workspace, hostId = connection.host.id, onDismiss = { describing = false })
    if (naming) NewWorktreeSheet(model = model, workspace = workspace, hostId = connection.host.id, onDismiss = { naming = false })
}

/** One level of the tree, pushed: its node's own page first, then its children. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TreeLevelScreen(connection: Connection, workspace: WorkspaceSummary, nodeId: String, nav: TreeNavigation, onBack: () -> Unit) {
    val prefs = LocalContext.current.getSharedPreferences(FILTER_PREFS, android.content.Context.MODE_PRIVATE)
    val filter = OneTree.Filter.parse(prefs.getString(filterKey(connection.host.id, workspace.id), null))
    val tree = rememberTree(connection, workspace, filter)
    val all = rememberTree(connection, workspace, OneTree.Filter.ALL)
    val node = tree?.node(nodeId) ?: all?.node(nodeId)
    Scaffold(topBar = {
        TopAppBar(
            title = { Text(node?.let { if (it.key.isEmpty()) it.title else "${it.key} ${it.title}" } ?: "", maxLines = 1, overflow = TextOverflow.Ellipsis) },
            navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") } },
        )
    }) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when {
                tree == null -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                node == null -> Column(Modifier.align(Alignment.Center).padding(32.dp).testTag("tree-gone"), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text("No longer here", style = MaterialTheme.typography.titleMedium)
                    Text("It finished or moved since this opened.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                else -> LazyColumn(Modifier.fillMaxSize().testTag("tree-level")) {
                    val own = OneTree.ownRow(node)
                    if (own != null && node.target != null) item(key = "own") {
                        ListItem(
                            headlineContent = { Text(own, color = MaterialTheme.colorScheme.primary) },
                            supportingContent = if (node.also.isNotEmpty()) ({ Text(node.also) }) else null,
                            leadingContent = { Icon(icon(node), null, tint = MaterialTheme.colorScheme.primary) },
                            trailingContent = { Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.outline) },
                            modifier = Modifier.clickable(role = Role.Button) { open(node.target, nav) }.testTag("tree-own"),
                        )
                        HorizontalDivider()
                    }
                    items(node.children, key = { it.id }) { TreeRow(it, nav) }
                }
            }
        }
    }
}

private fun open(target: OneTree.Target, nav: TreeNavigation) {
    when (target) {
        is OneTree.Target.Task -> nav.onOpenTask(target.id)
        is OneTree.Target.Theme -> nav.onOpenPlan(PlanPage.Theme(target.id))
        is OneTree.Target.Lane -> nav.onOpenPlan(PlanPage.Lane(target.id))
        is OneTree.Target.Worktree -> nav.onOpenWorktree(target.id)
        is OneTree.Target.Terminal -> nav.onOpenTerminal(target.worktree, target.terminal)
        OneTree.Target.Orchestrator -> Unit
    }
}

@Composable
internal fun TreeRow(node: OneTree.Node, nav: TreeNavigation) {
    val tap = OneTree.tap(node)
    val amber = glanceColor(GlancePalette.amber)
    val spoken = listOfNotNull(node.key.ifEmpty { null }, node.title, node.detail.ifEmpty { null }, node.also.ifEmpty { null }, node.caption.ifEmpty { null }, if (node.showsDot) "Needs you" else null).joinToString(", ")
    val modifier = when (tap) {
        is OneTree.Tap.Push -> Modifier.clickable(role = Role.Button) { nav.onOpenLevel(tap.node) }
        is OneTree.Tap.Open -> Modifier.clickable(role = Role.Button) { open(tap.target, nav) }
        OneTree.Tap.None -> Modifier
    }
    ListItem(
        leadingContent = { Icon(icon(node), null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) },
        headlineContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (node.key.isNotEmpty()) {
                    Text(node.key, style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Spacer(Modifier.width(8.dp))
                }
                Text(node.title, maxLines = 2, overflow = TextOverflow.Ellipsis, color = if (node.quiet) MaterialTheme.colorScheme.onSurfaceVariant else Color.Unspecified)
            }
        },
        supportingContent = listOf(node.also, node.caption).filter { it.isNotEmpty() }.takeIf { it.isNotEmpty() }?.let { lines ->
            { Column { lines.forEach { Text(it, style = MaterialTheme.typography.bodySmall) } } }
        },
        trailingContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (node.showsDot) {
                    Box(Modifier.size(8.dp).padding(0.dp)) { Surface(shape = CircleShape, color = amber, modifier = Modifier.fillMaxSize().testTag("tree-dot")) {} }
                    Spacer(Modifier.width(8.dp))
                }
                if (node.detail.isNotEmpty()) Text(node.detail, style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum"), color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (tap != OneTree.Tap.None) Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.outline)
            }
        },
        modifier = modifier.testTag("tree-row-${node.key.ifEmpty { node.title }}").semantics(mergeDescendants = true) { contentDescription = spoken },
    )
}

private fun icon(node: OneTree.Node): ImageVector = when (node.kind) {
    OneTree.Kind.THEME -> Icons.Outlined.Map
    OneTree.Kind.TASK -> Icons.Outlined.Circle
    OneTree.Kind.DONE_FOLD -> Icons.Outlined.CheckCircleOutline
    OneTree.Kind.LANE -> Icons.Outlined.AccountTree
    OneTree.Kind.TERMINAL -> if (node.detail == "Agent") Icons.Outlined.AutoAwesome else Icons.Outlined.Terminal
    OneTree.Kind.SUBAGENT -> Icons.Outlined.Person
    OneTree.Kind.WORKTREE -> Icons.Outlined.AccountTree
    OneTree.Kind.GROUP -> when (node.id) {
        "group:main" -> Icons.Outlined.Home
        "group:loose" -> Icons.Outlined.Inventory2
        else -> Icons.Outlined.Inbox
    }
}

/** Where [workspace]'s tree rows go, over [model]'s stack. */
fun treeNavigation(model: AppModel, hostId: String, workspaceId: String, worktrees: () -> List<com.farcooler.model.Worktree>) = TreeNavigation(
    onOpenTask = { model.navigate(Route.BoardTask(hostId, workspaceId, it)) },
    onOpenPlan = { model.navigate(Route.PlanPage(hostId, workspaceId, it.kind, it.id)) },
    onOpenTerminal = { worktree, terminal -> model.openFromBoard(com.farcooler.net.TerminalRef(hostId, worktree, terminal)) },
    onOpenWorktree = { id ->
        // On its first pane of its own, else on its changes: a worktree has
        // no page of its own but what's in it.
        val pane = worktrees().firstOrNull { it.id == id }?.terminals?.firstOrNull { !it.isOrchestrator }
        if (pane != null) model.openFromBoard(com.farcooler.net.TerminalRef(hostId, id, pane.id)) else model.openChanges(hostId, id)
    },
    onOpenLevel = { model.navigate(Route.TreeLevel(hostId, workspaceId, it)) },
)
