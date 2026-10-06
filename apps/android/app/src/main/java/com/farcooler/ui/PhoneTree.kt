package com.farcooler.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.combinedClickable
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.height
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.AccountTree
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.Circle
import androidx.compose.material.icons.outlined.Description
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

/**
 * What the plan says now: the strip, read from [connection]'s plan, board and
 * Needs You list, with the reads that fill them kicked off when nothing has
 * been read yet. The phone's strip and the wide workspace's plan home both
 * draw it.
 */
@Composable
internal fun rememberPlanStrip(connection: Connection, workspace: WorkspaceSummary, orchestrator: Terminal?): PlanStrip {
    val planStates by connection.plans.states.collectAsStateWithLifecycle()
    val boards by connection.boards.collectAsStateWithLifecycle()
    val list by connection.needsYou.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val keepsPlan = daemon?.can(Capability.BOARD_PLAN) == true
    LaunchedEffect(workspace.id, keepsPlan) {
        if (boards[workspace.id] == null) connection.readBoard(workspace)
        if (keepsPlan && planStates[workspace.id] == null) connection.plans.read(workspace)
    }
    val plan = (planStates[workspace.id] as? PlanReadState.Loaded)?.plan ?: Plan()
    return PlanStrip.of(plan, PlanStrip.needsYouCount(workspace, boards[workspace.id], plan, list), orchestrator)
}

/**
 * The plan's overview as list items: the orchestrator's header, then the
 * plan's own rows (or the notice that the runner needs an update). The
 * sheet's body on the phone and the main pane's home on a wide screen.
 * [onNeedsYou] and [onOpen] are what a tap on those rows does; the sheet
 * wraps them to close itself first.
 */
internal fun LazyListScope.planHome(
    connection: Connection,
    workspace: WorkspaceSummary,
    strip: PlanStrip,
    keepsPlan: Boolean,
    planState: PlanReadState?,
    statuses: Map<String, com.farcooler.model.TaskStatus>,
    rulings: RulingsHook?,
    scope: kotlinx.coroutines.CoroutineScope,
    onNeedsYou: () -> Unit,
    onOpen: (PlanPage) -> Unit,
) {
    item(key = "orchestrator") { SheetHeader(strip, onNeedsYou) }
    if (keepsPlan) {
        planItems(
            state = planState,
            statuses = statuses,
            onOpen = onOpen,
            onRetry = { scope.launch { connection.plans.read(workspace) } },
            // Decided For You's copy, as on the Board (review 5).
            rulings = rulings,
        )
    } else {
        item(key = "needs-update") { PlanNotice(com.farcooler.model.PlanWords.NEEDS_UPDATE, null) }
    }
}

/**
 * The plan as the main pane's home on a wide screen (ov-347): the sheet's
 * body without the sheet, so the plan is always open beside the chat.
 */
@Composable
fun PlanHome(
    connection: Connection,
    workspace: WorkspaceSummary,
    orchestrator: Terminal?,
    onOpenPlan: (PlanPage) -> Unit,
    onNeedsYou: () -> Unit,
    onJump: (com.farcooler.net.TerminalRef) -> Unit,
) {
    val planStates by connection.plans.states.collectAsStateWithLifecycle()
    val boards by connection.boards.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val keepsPlan = daemon?.can(Capability.BOARD_PLAN) == true
    val scope = rememberCoroutineScope()
    val rulings = rememberRulingsHook(connection, workspace, onJump)
    val strip = rememberPlanStrip(connection, workspace, orchestrator)
    LazyColumn(Modifier.fillMaxSize().testTag("plan-home")) {
        planHome(
            connection, workspace, strip, keepsPlan, planStates[workspace.id],
            boards[workspace.id]?.rows.orEmpty().associate { it.id to it.status },
            rulings, scope, onNeedsYou, onOpenPlan,
        )
    }
}

/** This navigation, running [after] first on every destination it opens. */
fun TreeNavigation.then(after: () -> Unit) = TreeNavigation(
    onOpenTask = { after(); onOpenTask(it) },
    onOpenPlan = { after(); onOpenPlan(it) },
    onOpenTerminal = { worktree, terminal -> after(); onOpenTerminal(worktree, terminal) },
    onOpenWorktree = { after(); onOpenWorktree(it) },
    onOpenLevel = { after(); onOpenLevel(it) },
)

/** The strip, under the tab row while the orchestrator is up, and the sheet it opens. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PlanStripBar(connection: Connection, workspace: WorkspaceSummary, orchestrator: Terminal?, onOpenPlan: (PlanPage) -> Unit, onNeedsYou: () -> Unit = {}, onJump: (com.farcooler.net.TerminalRef) -> Unit = {}) {
    var peekingJump by remember { mutableStateOf<com.farcooler.net.TerminalRef?>(null) }
    val planStates by connection.plans.states.collectAsStateWithLifecycle()
    val boards by connection.boards.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val keepsPlan = daemon?.can(Capability.BOARD_PLAN) == true
    val scope = rememberCoroutineScope()
    // Keep, Keep all, Reverse and Discuss, as on the Board; a jump to the
    // orchestrator's pane closes the sheet first.
    val rulings = rememberRulingsHook(connection, workspace) { ref ->
        peekingJump = ref
    }
    val strip = rememberPlanStrip(connection, workspace, orchestrator)
    var peeking by remember { mutableStateOf(false) }
    if (!strip.isEmpty) PlanStripPill(strip) { peeking = true }
    LaunchedEffect(peekingJump) {
        val ref = peekingJump ?: return@LaunchedEffect
        peeking = false
        peekingJump = null
        onJump(ref)
    }
    if (peeking) {
        val state = rememberModalBottomSheetState(skipPartiallyExpanded = false)
        ModalBottomSheet(onDismissRequest = { peeking = false }, sheetState = state, modifier = Modifier.testTag("plan-sheet")) {
            LazyColumn(Modifier.fillMaxWidth()) {
                planHome(
                    connection, workspace, strip, keepsPlan, planStates[workspace.id],
                    boards[workspace.id]?.rows.orEmpty().associate { it.id to it.status },
                    rulings, scope,
                    onNeedsYou = {
                        scope.launch {
                            state.hide()
                            peeking = false
                            onNeedsYou()
                        }
                    },
                    onOpen = { page ->
                        scope.launch {
                            state.hide()
                            peeking = false
                            onOpenPlan(page)
                        }
                    },
                )
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

/**
 * The parts, what needs you first and in weight, behind an amber dot. Not
 * amber text: on the pill it falls under the 4.5:1 that text needs (review
 * 17); the dot carries the color, the words the meaning.
 */
@Composable
private fun StripWords(strip: PlanStrip, modifier: Modifier) {
    val amber = glanceColor(GlancePalette.amber)
    val text = androidx.compose.ui.text.buildAnnotatedString {
        var rest = strip.parts
        strip.needsYouWords?.let { needs ->
            pushStyle(androidx.compose.ui.text.SpanStyle(color = amber))
            append("● ")
            pop()
            pushStyle(androidx.compose.ui.text.SpanStyle(fontWeight = FontWeight.SemiBold))
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
internal fun SheetHeader(strip: PlanStrip, onNeedsYou: () -> Unit = {}) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 8.dp).testTag("plan-sheet-orchestrator"), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(stateIcon(strip.orchestrator), null, Modifier.size(20.dp), tint = tone(strip.orchestrator.tone))
            Spacer(Modifier.width(10.dp))
            Text("Orchestrator · ${strip.orchestrator.word}", style = MaterialTheme.typography.titleMedium)
        }
        strip.line?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 3, overflow = TextOverflow.Ellipsis) }
        // The way to what needs you (review 12): the flag carries the color,
        // the words stay the text's own (review 17).
        strip.needsYouWords?.let {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.fillMaxWidth().clickable(role = Role.Button, onClick = onNeedsYou).padding(vertical = 6.dp).testTag("plan-sheet-needs-you"),
            ) {
                Icon(Icons.Outlined.Flag, null, Modifier.size(18.dp), tint = glanceColor(GlancePalette.amber))
                Spacer(Modifier.width(8.dp))
                Text(it, modifier = Modifier.weight(1f))
                Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.outline)
            }
        }
        Separator(Modifier.padding(top = 8.dp))
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
    val pageLists by connection.pages.lists.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val keepsPlan = daemon?.can(Capability.BOARD_PLAN) == true
    val keepsPages = daemon?.can(Capability.BOARD_PAGES) == true
    LaunchedEffect(workspace.id, keepsPlan) {
        if (boards[workspace.id] == null) connection.readBoard(workspace)
        if (keepsPlan && planStates[workspace.id] == null) connection.plans.read(workspace)
        if (keepsPages && pageLists[workspace.id] == null) connection.pages.read(workspace)
    }
    val board = boards[workspace.id] ?: return null
    val plan = (planStates[workspace.id] as? PlanReadState.Loaded)?.plan ?: Plan()
    val pages = (pageLists[workspace.id] as? com.farcooler.net.PageListState.Loaded)?.pages.orEmpty()
    return remember(board, plan, fleet, list, filter, pages) {
        OneTree.build(workspace, board, plan, fleet.worktrees, list?.items.orEmpty(), filter, pages)
    }
}

/** The Themes tab: the tree's root, under a row of filter chips. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ThemesTab(connection: Connection, workspace: WorkspaceSummary, nav: TreeNavigation) {
    val prefs = LocalContext.current.getSharedPreferences(FILTER_PREFS, android.content.Context.MODE_PRIVATE)
    val key = filterKey(connection.host.id, workspace.id)
    var filter by remember(key) { mutableStateOf(OneTree.Filter.parse(prefs.getString(key, null))) }
    val tree = rememberTree(connection, workspace, filter)
    val menu = rememberTreeWorktreeMenu(connection, workspace, tree)
    val unread by connection.unreadBoards.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    Column(Modifier.fillMaxSize()) {
        TreeFilterChips(filter) { f ->
            filter = f
            prefs.edit().putString(key, f.name).apply()
        }
        TreeRootList(tree, filter, failed = tree == null && workspace.id in unread, nav = nav, menu = menu) {
            scope.launch { connection.readBoard(workspace) }
        }
    }
    menu.Sheets()
}

/** Material's filter chips: one choice of three, kept per workspace (review 18). */
@Composable
internal fun TreeFilterChips(filter: OneTree.Filter, onChoose: (OneTree.Filter) -> Unit) {
    // Scrolls, for the wide workspace's 240 dp list pane, where the third chip
    // is past the edge; a fade at the edge says there's more.
    val scroll = androidx.compose.foundation.rememberScrollState()
    Box(Modifier.fillMaxWidth()) {
        Row(Modifier.fillMaxWidth().horizontalScroll(scroll).padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OneTree.Filter.entries.forEach { f ->
                androidx.compose.material3.FilterChip(
                    selected = f == filter,
                    onClick = { onChoose(f) },
                    label = { Text(f.title) },
                    modifier = Modifier.testTag("tree-filter-${f.name.lowercase()}"),
                )
            }
        }
        if (scroll.canScrollForward) {
            Box(
                Modifier.align(Alignment.CenterEnd).width(32.dp).height(FilterChipDefaults.Height).testTag("tree-filter-fade").background(
                    androidx.compose.ui.graphics.Brush.horizontalGradient(
                        listOf(Color.Transparent, LocalTreeFade.current.takeIf { it != Color.Unspecified } ?: MaterialTheme.colorScheme.surface),
                    ),
                ),
            )
        }
    }
}

/** The root's rows, or why there are none. */
@Composable
internal fun TreeRootList(tree: OneTree.Tree?, filter: OneTree.Filter, failed: Boolean, nav: TreeNavigation, menu: TreeWorktreeMenu?, onRetry: () -> Unit) {
    when {
        // A read that failed says so, with Try again, never a spinner for good (review 15).
        failed -> Column(Modifier.fillMaxWidth().padding(16.dp).testTag("tree-board-failed")) {
            Text("Far Cooler couldn’t read this board.", color = MaterialTheme.colorScheme.onSurfaceVariant)
            androidx.compose.material3.TextButton(onClick = onRetry, modifier = Modifier.testTag("tree-board-retry")) { Text(com.farcooler.model.PlanWords.TRY_AGAIN) }
        }
        tree == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator(Modifier.testTag("tree-loading")) }
        else -> LazyColumn(Modifier.fillMaxSize().testTag("tree-root")) {
            if (tree.work.isEmpty()) item(key = "empty") {
                Text(
                    if (filter == OneTree.Filter.IN_REVIEW) "Nothing is in review." else "No cards yet. The orchestrator files them as it plans the work.",
                    color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(16.dp).testTag("tree-empty"),
                )
            }
            items(tree.work, key = { it.id }) { TreeRow(it, nav, menu) }
            if (tree.below.isNotEmpty()) item(key = "divider") { Separator(Modifier.padding(vertical = 8.dp)) }
            items(tree.below, key = { it.id }) { TreeRow(it, nav, menu) }
        }
    }
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
    val menu = rememberTreeWorktreeMenu(connection, workspace, if (tree?.node(nodeId) != null) tree else all)
    val unread by connection.unreadBoards.collectAsStateWithLifecycle()
    val planStates by connection.plans.states.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val boardRead = when {
        tree != null -> OneTree.Read.READ
        workspace.id in unread -> OneTree.Read.FAILED
        else -> OneTree.Read.PENDING
    }
    val planRead = when {
        daemon?.can(Capability.BOARD_PLAN) != true -> OneTree.Read.NOT_KEPT
        else -> when (planStates[workspace.id]) {
            null, PlanReadState.Loading -> OneTree.Read.PENDING
            PlanReadState.Unavailable -> OneTree.Read.FAILED
            else -> OneTree.Read.READ
        }
    }
    val state = OneTree.level(node != null, boardRead, planRead)
    Scaffold(topBar = {
        TopAppBar(
            title = { Text(node?.let { if (it.key.isEmpty()) it.title else "${it.key} ${it.title}" } ?: WorkspaceTab.WORKTREES.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") } },
        )
    }) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            when (state) {
                // Restored before the board and the plan were read: it waits (review 1).
                OneTree.LevelState.LOADING -> CircularProgressIndicator(Modifier.align(Alignment.Center).testTag("tree-level-loading"))
                OneTree.LevelState.FAILED -> Column(Modifier.align(Alignment.Center).padding(32.dp).testTag("tree-level-failed"), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(com.farcooler.model.PlanWords.COULDNT_READ, style = MaterialTheme.typography.titleMedium)
                    androidx.compose.material3.TextButton(onClick = {
                        scope.launch {
                            connection.readBoard(workspace)
                            if (daemon?.can(Capability.BOARD_PLAN) == true) connection.plans.read(workspace)
                        }
                    }) { Text(com.farcooler.model.PlanWords.TRY_AGAIN) }
                }
                OneTree.LevelState.GONE -> Column(Modifier.align(Alignment.Center).padding(32.dp).testTag("tree-gone"), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text("No longer here", style = MaterialTheme.typography.titleMedium)
                    Text("It finished or moved since this opened.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                OneTree.LevelState.NODE -> if (node != null) LazyColumn(Modifier.fillMaxSize().testTag("tree-level")) {
                    val own = OneTree.ownRow(node)
                    if (own != null && node.target != null) item(key = "own") {
                        ListItem(
                            headlineContent = { Text(own, color = MaterialTheme.colorScheme.primary) },
                            supportingContent = if (node.also.isNotEmpty()) ({ Text(node.also) }) else null,
                            leadingContent = { Icon(icon(node), null, tint = MaterialTheme.colorScheme.primary) },
                            trailingContent = { Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.outline) },
                            modifier = Modifier.clickable(role = Role.Button) { open(node.target, nav) }.testTag("tree-own"),
                        )
                        Separator()
                    }
                    items(node.children, key = { it.id }) { TreeRow(it, nav, menu) }
                }
            }
        }
    }
    menu.Sheets()
}

private fun open(target: OneTree.Target, nav: TreeNavigation) {
    when (target) {
        is OneTree.Target.Task -> nav.onOpenTask(target.id)
        is OneTree.Target.Theme -> nav.onOpenPlan(PlanPage.Theme(target.id))
        is OneTree.Target.Lane -> nav.onOpenPlan(PlanPage.Lane(target.id))
        is OneTree.Target.Worktree -> nav.onOpenWorktree(target.id)
        is OneTree.Target.Terminal -> nav.onOpenTerminal(target.worktree, target.terminal)
        is OneTree.Target.Page -> nav.onOpenPlan(PlanPage.Page(target.slot))
        OneTree.Target.Orchestrator -> Unit
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun TreeRow(node: OneTree.Node, nav: TreeNavigation, menu: TreeWorktreeMenu? = null) {
    val tap = OneTree.tap(node)
    val actions = menu?.actions(node).orEmpty()
    var menuOpen by remember { mutableStateOf(false) }
    val amber = glanceColor(GlancePalette.amber)
    val spoken = listOfNotNull(node.key.ifEmpty { null }, node.title, node.detail.ifEmpty { null }, node.also.ifEmpty { null }, node.caption.ifEmpty { null }, if (node.showsDot) "Needs you" else null).joinToString(", ")
    val onClick: (() -> Unit)? = when (tap) {
        is OneTree.Tap.Push -> { { nav.onOpenLevel(tap.node) } }
        is OneTree.Tap.Open -> { { open(tap.target, nav) } }
        OneTree.Tap.None -> null
    }
    // A worktree's menu on a long press, as the Worktrees tab's rows held
    // theirs behind a press; each action is TalkBack's too.
    val modifier = when {
        actions.isNotEmpty() -> Modifier.combinedClickable(role = Role.Button, onClick = onClick ?: {}, onLongClick = { menuOpen = true })
            .semantics { customActions = actions.map { a -> CustomAccessibilityAction(a.title) { menu?.perform(node, a); true } } }
        onClick != null -> Modifier.clickable(role = Role.Button, onClick = onClick)
        else -> Modifier
    }
    Box {
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
        supportingContent = listOf(node.also, node.caption, menu?.worktreeOf(node)?.branch.orEmpty()).filter { it.isNotEmpty() }.takeIf { it.isNotEmpty() }?.let { lines ->
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
    DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }, modifier = Modifier.testTag("tree-row-menu")) {
        actions.forEach { action ->
            if (action == com.farcooler.model.WorktreeAction.REMOVE) Separator()
            DropdownMenuItem(
                text = { Text(action.title, color = if (action == com.farcooler.model.WorktreeAction.REMOVE) MaterialTheme.colorScheme.error else Color.Unspecified) },
                onClick = {
                    menuOpen = false
                    menu?.perform(node, action)
                },
            )
        }
    }
    }
}

private fun icon(node: OneTree.Node): ImageVector = when (node.kind) {
    OneTree.Kind.THEME -> Icons.Outlined.Map
    OneTree.Kind.TASK -> Icons.Outlined.Circle
    OneTree.Kind.DONE_FOLD -> Icons.Outlined.CheckCircleOutline
    OneTree.Kind.LANE -> Icons.Outlined.AccountTree
    OneTree.Kind.TERMINAL -> if (node.detail == "Agent") Icons.Outlined.AutoAwesome else Icons.Outlined.Terminal
    OneTree.Kind.SUBAGENT -> Icons.Outlined.Person
    OneTree.Kind.PAGE -> Icons.Outlined.Description
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

/**
 * The worktree menu on the tree's rows (ov-300): what the Worktrees tab's
 * header offered, kept now that the tree has taken its place. A long press on
 * a lane's, a loose worktree's or the checkout's row opens it, and TalkBack
 * reads each as a custom action. The actions are `WorktreeActions.of`, the
 * rule the Worktrees list reads too.
 */
class TreeWorktreeMenu internal constructor(
    private val connection: Connection,
    private val workspace: WorkspaceSummary,
    private val tree: OneTree.Tree?,
    private val scope: kotlinx.coroutines.CoroutineScope,
) {
    internal var newTerminal by mutableStateOf<com.farcooler.model.Worktree?>(null)
    internal var stack by mutableStateOf<com.farcooler.model.Worktree?>(null)
    internal var removing by mutableStateOf<com.farcooler.model.Worktree?>(null)

    /** The row's worktree: a lane's, a loose one's, or the checkout. */
    fun worktreeOf(node: OneTree.Node): com.farcooler.model.Worktree? = worktree(node)

    private fun worktree(node: OneTree.Node): com.farcooler.model.Worktree? {
        if (node.kind != OneTree.Kind.LANE && node.kind != OneTree.Kind.WORKTREE && node.id != "group:main") return null
        val id = node.worktreeId ?: return null
        return connection.fleet.value.worktrees.firstOrNull { it.id == id }
    }

    fun actions(node: OneTree.Node): List<com.farcooler.model.WorktreeAction> {
        val wt = worktree(node) ?: return emptyList()
        val (above, below) = tree?.let { OneTree.neighbors(it, node.id) } ?: (null to null)
        return com.farcooler.model.WorktreeActions.of(wt, above, below, workspace.id)
    }

    fun perform(node: OneTree.Node, action: com.farcooler.model.WorktreeAction) {
        val wt = worktree(node) ?: return
        when (action) {
            com.farcooler.model.WorktreeAction.NEW_TERMINAL -> newTerminal = wt
            com.farcooler.model.WorktreeAction.STACK -> stack = wt
            com.farcooler.model.WorktreeAction.REMOVE -> removing = wt
            com.farcooler.model.WorktreeAction.HIDE, com.farcooler.model.WorktreeAction.UNHIDE ->
                scope.launch { connection.setHidden(wt, !wt.isHidden) }
            com.farcooler.model.WorktreeAction.MOVE_UP, com.farcooler.model.WorktreeAction.MOVE_DOWN -> {
                val (above, below) = tree?.let { OneTree.neighbors(it, node.id) } ?: (null to null)
                val up = action == com.farcooler.model.WorktreeAction.MOVE_UP
                val beside = (if (up) above else below) ?: return
                // The runner's order of this workspace's worktrees, as the
                // Worktrees list sent it from a drag.
                val fleet = connection.fleet.value
                val order = fleet.worktrees.filter { com.farcooler.model.WorktreeScope.OfWorkspace(connection.host.id, workspace).includes(it, fleet) }.map { it.id }
                val next = com.farcooler.model.WorktreeActions.moved(order, wt.id, beside, up)
                if (next != order) scope.launch { connection.reorderWorktrees(next) }
            }
        }
    }

    @Composable
    fun Sheets() {
        newTerminal?.let { NewTerminalSheet(connection = connection, worktree = it, onDismiss = { newTerminal = null }) }
        stack?.let { wt ->
            wt.repository?.let { StackSheet(connection = connection, repository = it, branch = wt.branch, onDismiss = { stack = null }) }
        }
        removing?.let { RemoveWorktreeCeremony(connection = connection, worktree = it, onFinished = { removing = null }) }
    }
}

@Composable
fun rememberTreeWorktreeMenu(connection: Connection, workspace: WorkspaceSummary, tree: OneTree.Tree?): TreeWorktreeMenu {
    val scope = rememberCoroutineScope()
    return remember(connection, workspace, tree) { TreeWorktreeMenu(connection, workspace, tree, scope) }
}
