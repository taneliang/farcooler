@file:OptIn(ExperimentalMaterial3AdaptiveApi::class)

package com.farcooler.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Flag
import androidx.compose.material.icons.outlined.AccountTree
import androidx.compose.material.icons.outlined.ViewKanban
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
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
 * Which workspace layout a window gets (ov-347): the phone's tabs, or the wide
 * one with a rail, the tree, the plan and the orchestrator's chat side by side.
 *
 * The line is Material's expanded width, 840 dp. A tablet is past it in either
 * orientation it is held in a hand, and a foldable is past it only unfolded:
 * folded, the window is narrow and this says [Kind.PHONE], which is what the
 * design asks of it. The width read is the WINDOW's, not the screen's, so a
 * tablet in split screen is a phone for as long as it is narrow.
 *
 * A runner without workspaces has no orchestrator and no plan to put beside a
 * tree, so its workspace stays the phone's Board and Worktrees however wide
 * the window is.
 */
object WorkspaceLayout {
    /** Material's expanded window width class begins here. */
    const val EXPANDED_DP = 840

    enum class Kind { PHONE, WIDE }

    fun of(widthDp: Int, implicit: Boolean): Kind =
        if (!implicit && widthDp >= EXPANDED_DP) Kind.WIDE else Kind.PHONE

    /** The current window's, read the way the adaptive library reads it. */
    @Composable
    fun current(implicit: Boolean): Kind {
        val width = LocalWindowInfo.current.containerDpSize.width
        return of(width.value.toInt(), implicit)
    }
}

/** The three panes of the wide workspace, in the scaffold's roles. */
enum class WidePane(val tag: String) {
    /** The leading column. */
    LIST("wide-pane-list"),

    /** The middle, taking whatever width the other two leave. */
    MAIN("wide-pane-main"),

    /** The trailing column, the scaffold's supporting pane. */
    SUPPORTING("wide-pane-supporting"),
}

/** What a pane holds. */
enum class WideContent { TREE, PLAN, BOARD, CHAT }

/**
 * The rail's two places. The tree and the chat are in both: the rail changes
 * only what the main pane is, which is the plan's home or the board.
 *
 * Needs You and Back are the rail's own actions rather than places in this
 * workspace, so they aren't here.
 */
enum class WideDestination(val title: String, val tab: WorkspaceTab) {
    /** The Orchestrator and Themes tabs both fold into this one: the chat and the tree are always up. */
    PLAN("Plan", WorkspaceTab.ORCHESTRATOR),
    BOARD("Board", WorkspaceTab.BOARD);

    /** What each pane holds while this destination is selected. */
    fun contents(): Map<WidePane, WideContent> = mapOf(
        WidePane.LIST to WideContent.TREE,
        WidePane.MAIN to if (this == PLAN) WideContent.PLAN else WideContent.BOARD,
        WidePane.SUPPORTING to WideContent.CHAT,
    )

    companion object {
        /** The place a remembered or pushed tab lands on a wide screen. */
        fun of(tab: WorkspaceTab): WideDestination = if (tab == WorkspaceTab.BOARD) BOARD else PLAN
    }
}

/**
 * The scaffold's value for the wide layout: every pane expanded. Constant on
 * purpose. The chat is "always shown", and the layout is only used past 840 dp,
 * where the three columns fit; nothing here navigates between panes.
 */
internal val WideScaffoldValue = ThreePaneScaffoldValue(
    primary = PaneAdaptedValue.Expanded,
    secondary = PaneAdaptedValue.Expanded,
    tertiary = PaneAdaptedValue.Expanded,
)

/** Three horizontal partitions, no gutter between them: each pane is its own surface. */
internal val WideScaffoldDirective = PaneScaffoldDirective(
    maxHorizontalPartitions = 3,
    horizontalPartitionSpacerSize = 0.dp,
    maxVerticalPartitions = 1,
    verticalPartitionSpacerSize = 0.dp,
    defaultPanePreferredWidth = 320.dp,
    excludedBounds = emptyList(),
)

/** The tree's column, and the chat's. The plan takes what's left. */
internal val WideListWidth: Dp = 240.dp
internal val WideChatWidth: Dp = 320.dp

/**
 * The wide workspace's frame: a navigation rail, and beside it Material's
 * list-detail scaffold with the tree as the list pane, the plan (or the
 * board) as the detail, and the orchestrator's chat as the extra pane, which
 * is the supporting pane of the design.
 *
 * What goes in each pane is [WideDestination.contents]; [content] draws it.
 * The frame knows nothing about connections, so the tests and the captures
 * can drive it with the app's own views.
 */
@Composable
fun WideWorkspaceFrame(
    destination: WideDestination,
    onSelect: (WideDestination) -> Unit,
    onBack: () -> Unit,
    onNeedsYou: () -> Unit,
    topBar: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    content: @Composable (WideContent) -> Unit,
) {
    val contents = destination.contents()
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
                label = { Text("Needs You") },
                modifier = Modifier.testTag("wide-rail-needs-you"),
            )
            NavigationRailItem(
                selected = destination == WideDestination.PLAN,
                onClick = { onSelect(WideDestination.PLAN) },
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
        Column(Modifier.weight(1f).fillMaxHeight()) {
            topBar()
            ListDetailPaneScaffold(
                directive = WideScaffoldDirective,
                value = WideScaffoldValue,
                listPane = {
                    AnimatedPane(Modifier.preferredWidth(WideListWidth)) {
                        Box(Modifier.fillMaxSize().testTag(WidePane.LIST.tag)) { content(contents.getValue(WidePane.LIST)) }
                    }
                },
                detailPane = {
                    AnimatedPane {
                        Box(Modifier.fillMaxSize().testTag(WidePane.MAIN.tag)) { content(contents.getValue(WidePane.MAIN)) }
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
