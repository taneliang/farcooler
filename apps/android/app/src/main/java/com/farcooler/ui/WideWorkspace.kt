@file:OptIn(ExperimentalMaterial3AdaptiveApi::class)

package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Flag
import androidx.compose.material.icons.outlined.AccountTree
import androidx.compose.material.icons.outlined.ViewKanban
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.adaptive.ExperimentalMaterial3AdaptiveApi
import androidx.compose.material3.adaptive.layout.AnimatedPane
import androidx.compose.material3.adaptive.layout.ListDetailPaneScaffold
import androidx.compose.material3.adaptive.layout.PaneAdaptedValue
import androidx.compose.material3.adaptive.layout.PaneScaffoldDirective
import androidx.compose.material3.adaptive.layout.ThreePaneScaffoldValue
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * Which workspace layout a window gets (ov-347): the phone's tabs, a rail with
 * the plan and the chat, or all three columns with the tree as well.
 *
 * Two lines, both Material's own width breakpoints. At 840 dp (expanded) the
 * rail, the plan and the chat fit, and a tablet is past it in either
 * orientation it is held in a hand; a foldable is past it only unfolded.
 * Folded, the window is narrow and this says [Kind.PHONE]. At 1200 dp (extra
 * large) the tree fits as a column as well. Between them the tree is folded
 * into the rail's Plan place, so that the plan keeps at least [MIN_PLAN_DP]
 * (owner ruling): at 840 dp it gets 440, and at 1200 dp 560.
 *
 * The width read is the WINDOW's, not the screen's, so a tablet in split
 * screen is a phone for as long as it is narrow.
 *
 * A runner without workspaces has no orchestrator and no plan to put beside a
 * tree, so its workspace stays the phone's Board and Worktrees however wide
 * the window is.
 */
object WorkspaceLayout {
    /** Material's expanded window width class begins here. */
    const val EXPANDED_DP = 840

    /** Material's extra-large width class begins here: room for the tree as a column. */
    const val THREE_PANE_DP = 1200

    /** What the plan pane is never given less than. */
    const val MIN_PLAN_DP = 360

    enum class Kind {
        PHONE,

        /** Rail, plan and chat; the tree opens over the plan from the rail's Plan place. */
        TWO_PANE,

        /** Rail, tree, plan and chat. */
        THREE_PANE,
    }

    fun of(widthDp: Int, implicit: Boolean): Kind = when {
        implicit || widthDp < EXPANDED_DP -> Kind.PHONE
        widthDp < THREE_PANE_DP -> Kind.TWO_PANE
        else -> Kind.THREE_PANE
    }

    /** The current window's, read the way the adaptive library reads it. */
    @Composable
    fun current(implicit: Boolean): Kind {
        val width = LocalWindowInfo.current.containerDpSize.width
        return of(width.value.toInt(), implicit)
    }
}

/** The panes of the wide workspace, in the scaffold's roles. */
enum class WidePane(val tag: String) {
    /** The leading column, or the panel that opens over the plan. */
    LIST("wide-pane-list"),

    /** The middle, taking whatever width the other two leave. */
    MAIN("wide-pane-main"),

    /** The trailing column, the scaffold's supporting pane. */
    SUPPORTING("wide-pane-supporting"),
}

/** What a pane holds. */
enum class WideContent { TREE, PLAN, BOARD, CHAT }

/**
 * The rail's two places. The chat is in both, and the tree is reachable from
 * both layouts; the rail changes only what the main pane is, which is the
 * plan's home or the board.
 *
 * Needs You and Back are the rail's own actions rather than places in this
 * workspace, so they aren't here.
 */
enum class WideDestination(val title: String, val tab: WorkspaceTab) {
    /** The Orchestrator and Themes tabs both fold into this one: the chat and the tree are always reachable. */
    PLAN("Plan", WorkspaceTab.ORCHESTRATOR),
    BOARD("Board", WorkspaceTab.BOARD);

    /**
     * What each pane holds while this destination is selected, at [kind]. The
     * chat is always shown. The tree is a column in [WorkspaceLayout.Kind.THREE_PANE]
     * and, in [WorkspaceLayout.Kind.TWO_PANE], a panel over the plan while
     * [treeOpen]. The phone has no panes.
     */
    fun contents(kind: WorkspaceLayout.Kind, treeOpen: Boolean = false): Map<WidePane, WideContent> = buildMap {
        if (kind == WorkspaceLayout.Kind.PHONE) return@buildMap
        if (kind == WorkspaceLayout.Kind.THREE_PANE || treeOpen) put(WidePane.LIST, WideContent.TREE)
        put(WidePane.MAIN, if (this@WideDestination == PLAN) WideContent.PLAN else WideContent.BOARD)
        put(WidePane.SUPPORTING, WideContent.CHAT)
    }

    companion object {
        /** The place a remembered or pushed tab lands on a wide screen. */
        fun of(tab: WorkspaceTab): WideDestination = if (tab == WorkspaceTab.BOARD) BOARD else PLAN
    }
}

/**
 * The scaffold's value: the chat and the plan expanded, and the list pane too
 * only when it is a column. Constant per layout, on purpose: nothing here
 * navigates between panes.
 */
internal fun wideScaffoldValue(kind: WorkspaceLayout.Kind) = ThreePaneScaffoldValue(
    primary = PaneAdaptedValue.Expanded,
    secondary = if (kind == WorkspaceLayout.Kind.THREE_PANE) PaneAdaptedValue.Expanded else PaneAdaptedValue.Hidden,
    tertiary = PaneAdaptedValue.Expanded,
)

/** Horizontal partitions as many as the columns shown, with no gutter: a separator divides them. */
internal fun wideScaffoldDirective(kind: WorkspaceLayout.Kind) = PaneScaffoldDirective(
    maxHorizontalPartitions = if (kind == WorkspaceLayout.Kind.THREE_PANE) 3 else 2,
    horizontalPartitionSpacerSize = 0.dp,
    maxVerticalPartitions = 1,
    verticalPartitionSpacerSize = 0.dp,
    defaultPanePreferredWidth = 320.dp,
    excludedBounds = emptyList(),
)

/** The tree's column, and the chat's. The plan takes what's left. */
internal val WideListWidth: Dp = 240.dp
internal val WideChatWidth: Dp = 320.dp

/** The rail's width, Material's. */
internal const val WideRailDp = 80

/**
 * The wide workspace's frame: a navigation rail, and beside it Material's
 * list-detail scaffold with the plan (or the board) as the detail pane and
 * the orchestrator's chat as the extra pane, which is the supporting pane of
 * the design. At [WorkspaceLayout.Kind.THREE_PANE] the tree is the list pane;
 * at [WorkspaceLayout.Kind.TWO_PANE] it is a panel over the plan's leading
 * edge, opened by tapping the rail's Plan place while it is selected.
 *
 * Panes are divided by the theme's separator, drawn as vertical rules.
 *
 * What goes in each pane is [WideDestination.contents]; [content] draws it.
 * The frame knows nothing about connections, so the tests and the captures
 * can drive it with the app's own views.
 */
@Composable
fun WideWorkspaceFrame(
    kind: WorkspaceLayout.Kind,
    destination: WideDestination,
    treeOpen: Boolean,
    onSelect: (WideDestination) -> Unit,
    onToggleTree: () -> Unit,
    onBack: () -> Unit,
    onNeedsYou: () -> Unit,
    topBar: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable (WideContent) -> Unit,
) {
    val contents = destination.contents(kind, treeOpen)
    val columns = kind == WorkspaceLayout.Kind.THREE_PANE
    Row(modifier.fillMaxSize().testTag("wide-workspace")) {
        NavigationRail(
            header = {
                IconButton(onClick = onBack, modifier = Modifier.testTag("wide-rail-back")) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                }
            },
            modifier = Modifier.testTag("wide-rail"),
        ) {
            NavigationRailItem(
                selected = false,
                onClick = onNeedsYou,
                icon = { Icon(Icons.Filled.Flag, contentDescription = null) },
                label = { Text("Needs you") },
                modifier = Modifier.testTag("wide-rail-needs-you"),
            )
            NavigationRailItem(
                selected = destination == WideDestination.PLAN,
                // Tapping the place that is already up opens the tree, where the tree isn't a column.
                onClick = { if (destination == WideDestination.PLAN && !columns) onToggleTree() else onSelect(WideDestination.PLAN) },
                icon = { Icon(Icons.Outlined.AccountTree, contentDescription = null) },
                label = { Text(WideDestination.PLAN.title) },
                modifier = Modifier.testTag("wide-rail-plan"),
            )
            NavigationRailItem(
                selected = destination == WideDestination.BOARD,
                onClick = { onSelect(WideDestination.BOARD) },
                icon = { Icon(Icons.Outlined.ViewKanban, contentDescription = null) },
                label = { Text(WideDestination.BOARD.title) },
                modifier = Modifier.testTag("wide-rail-board"),
            )
        }
        VerticalSeparator()
        Column(Modifier.weight(1f).fillMaxHeight()) {
            topBar()
            ListDetailPaneScaffold(
                directive = wideScaffoldDirective(kind),
                value = wideScaffoldValue(kind),
                listPane = {
                    AnimatedPane(Modifier.preferredWidth(WideListWidth)) {
                        Row(Modifier.fillMaxSize().testTag(WidePane.LIST.tag)) {
                            Box(Modifier.weight(1f).fillMaxHeight()) { content(contents.getValue(WidePane.LIST)) }
                            VerticalSeparator()
                        }
                    }
                },
                detailPane = {
                    AnimatedPane {
                        Row(Modifier.fillMaxSize()) {
                            Box(Modifier.weight(1f).fillMaxHeight().testTag(WidePane.MAIN.tag)) {
                                content(contents.getValue(WidePane.MAIN))
                                // The tree over the plan's leading edge, with a scrim that closes it.
                                if (!columns && WidePane.LIST in contents) {
                                    Box(
                                        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.scrim.copy(alpha = 0.32f))
                                            .clickable(onClick = onToggleTree).testTag("wide-tree-scrim"),
                                    )
                                    Surface(
                                        modifier = Modifier.width(WideListWidth).fillMaxHeight().testTag(WidePane.LIST.tag),
                                        tonalElevation = 3.dp,
                                        shadowElevation = 6.dp,
                                    ) {
                                        Row(Modifier.fillMaxSize()) {
                                            Box(Modifier.weight(1f).fillMaxHeight()) { content(contents.getValue(WidePane.LIST)) }
                                            VerticalSeparator()
                                        }
                                    }
                                }
                            }
                            VerticalSeparator()
                        }
                    }
                },
                extraPane = {
                    AnimatedPane(Modifier.preferredWidth(WideChatWidth)) {
                        Box(Modifier.fillMaxSize().testTag(WidePane.SUPPORTING.tag)) { content(contents.getValue(WidePane.SUPPORTING)) }
                    }
                },
            )
        }
    }
}
