package com.farcooler.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.AccountTree
import androidx.compose.material.icons.outlined.Map
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.foundation.clickable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.model.Capability
import com.farcooler.model.LaneTree
import com.farcooler.model.OneTree
import com.farcooler.model.Plan
import com.farcooler.model.PlanPage
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanSections
import com.farcooler.model.ThemeTree
import com.farcooler.model.WorkspaceSummary
import com.farcooler.net.Connection

// The two sidebar lists, read from a connection (ov-503, design 7). On a phone the Themes tab's root has two rows,
// Themes and Lanes, each pushing its list; on a tablet both lists stand in the list pane, one over the other.
// `SidebarLists.kt` draws the rows; this reads the plan and the board and hands them over.

/** Both lists' models, built over one reading of the plan. */
class SidebarModels(val themes: ThemeTree, val lanes: LaneTree)

/** [workspace]'s two lists as [connection] holds them, under [filter] (which applies to Themes only); null until the board is read. */
@Composable
internal fun rememberSidebarModels(connection: Connection, workspace: WorkspaceSummary, filter: OneTree.Filter): SidebarModels? {
    val planStates by connection.plans.states.collectAsStateWithLifecycle()
    val boards by connection.boards.collectAsStateWithLifecycle()
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val keepsPlan = daemon?.can(Capability.BOARD_PLAN) == true
    LaunchedEffect(workspace.id, keepsPlan) {
        if (boards[workspace.id] == null) connection.readBoard(workspace)
        if (keepsPlan && planStates[workspace.id] == null) connection.plans.read(workspace)
    }
    val board = boards[workspace.id] ?: return null
    val plan = (planStates[workspace.id] as? PlanReadState.Loaded)?.plan ?: Plan()
    return remember(board, plan, filter) {
        val statuses = board.rows.associate { it.id to it.status }
        SidebarModels(ThemeTree.of(plan, filter, statuses), LaneTree.of(plan, PlanSections.of(plan, statuses = statuses)))
    }
}

/** What a row in either list does, over [nav] and the runner's panes. */
@Composable
internal fun rememberSidebarActions(connection: Connection, nav: TreeNavigation): SidebarActions {
    val fleet by connection.fleet.collectAsStateWithLifecycle()
    return SidebarActions(
        onOpenTask = nav.onOpenTask,
        onOpenLane = { nav.onOpenPlan(PlanPage.Lane(it)) },
        onOpenTheme = { nav.onOpenPlan(PlanPage.Theme(it)) },
        onOpenTerminal = { terminal ->
            val worktree = fleet.worktrees.firstOrNull { wt -> wt.terminals.any { it.id == terminal } }
            if (worktree != null) nav.onOpenTerminal(worktree.id, terminal)
            worktree != null
        },
    )
}

/** Both lists in the tablet's list pane: Themes under its filter, then Lanes, then [below] (the checkout and loose worktrees). */
internal fun LazyListScope.sidebarPaneItems(
    models: SidebarModels,
    filterChips: @Composable () -> Unit,
    folds: PlanFolds,
    choice: SidebarChoice?,
    actions: SidebarActions,
    onChoose: (SidebarChoice) -> Unit,
) {
    item(key = "sidebar/themes") { SidebarListTitle("Themes", Modifier.testTag("sidebar-themes")) }
    item(key = "sidebar/filter") { filterChips() }
    themeListItems(models.themes, folds, choice, models.lanes, actions, onChoose)
    item(key = "sidebar/lanes") { SidebarListTitle("Lanes", Modifier.testTag("sidebar-lanes")) }
    laneListItems(models.lanes, models.themes, folds, choice, actions, onChoose)
}

/** The two rows at the phone's tree root: each pushes its list. */
internal fun LazyListScope.sidebarRootRows(models: SidebarModels?, onOpenLevel: (String) -> Unit) {
    val themes = models?.themes?.themes?.size
    val lanes = models?.lanes?.groups?.sumOf { it.lanes.size }
    item(key = "sidebar/row/themes") {
        ListItem(
            leadingContent = { Icon(Icons.Outlined.Map, null, tint = MaterialTheme.colorScheme.onSurfaceVariant) },
            headlineContent = { Text("Themes") },
            supportingContent = themes?.let { { Text(if (it == 1) "1 theme" else "$it themes") } },
            trailingContent = { Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.outline) },
            modifier = Modifier.clickable(role = Role.Button) { onOpenLevel(THEMES_LIST) }.testTag("sidebar-row-themes"),
        )
    }
    item(key = "sidebar/row/lanes") {
        ListItem(
            leadingContent = { Icon(Icons.Outlined.AccountTree, null, tint = MaterialTheme.colorScheme.onSurfaceVariant) },
            headlineContent = { Text("Lanes") },
            supportingContent = lanes?.let { { Text(if (it == 1) "1 lane" else "$it lanes") } },
            trailingContent = { Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.outline) },
            modifier = Modifier.clickable(role = Role.Button) { onOpenLevel(LANES_LIST) }.testTag("sidebar-row-lanes"),
        )
    }
}

/** The node ids a pushed list goes by, in the tree level's route. */
const val THEMES_LIST = "list:themes"
const val LANES_LIST = "list:lanes"

/** A pushed list: Themes (under its filter) or Lanes, with its title and a way back. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SidebarListScreen(connection: Connection, workspace: WorkspaceSummary, list: String, nav: TreeNavigation, onBack: () -> Unit) {
    val prefs = LocalContext.current.getSharedPreferences(FILTER_PREFS, android.content.Context.MODE_PRIVATE)
    val key = filterKey(connection.host.id, workspace.id)
    var filter by remember(key) { mutableStateOf(OneTree.Filter.parse(prefs.getString(key, null))) }
    val models = rememberSidebarModels(connection, workspace, filter)
    val actions = rememberSidebarActions(connection, nav)
    val folds = rememberPlanFolds(workspace.id to list)
    var choice by remember { mutableStateOf<SidebarChoice?>(null) }
    val themes = list == THEMES_LIST
    Scaffold(topBar = {
        TopAppBar(
            title = { Text(if (themes) "Themes" else "Lanes") },
            navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") } },
        )
    }) { padding ->
        Box(Modifier.fillMaxSize().padding(padding)) {
            if (models == null) CircularProgressIndicator(Modifier.align(Alignment.Center).testTag("tree-level-loading"))
            else LazyColumn(Modifier.fillMaxSize().testTag(if (themes) "themes-list" else "lanes-list")) {
                if (themes) {
                    item(key = "filter") {
                        TreeFilterChips(filter) { f ->
                            filter = f
                            prefs.edit().putString(key, f.name).apply()
                        }
                    }
                    themeListItems(models.themes, folds, choice, models.lanes, actions) { choice = it }
                } else {
                    laneListItems(models.lanes, models.themes, folds, choice, actions) { choice = it }
                }
            }
        }
    }
}
