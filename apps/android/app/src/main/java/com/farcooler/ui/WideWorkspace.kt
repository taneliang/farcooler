@file:OptIn(ExperimentalMaterial3AdaptiveApi::class)

package com.farcooler.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.focusGroup
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
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

/** The tree's column. */
internal val WideListWidth: Dp = 240.dp

/**
 * The chat's column: 360 dp where there are three columns to spare it, 320 dp
 * beside only the plan, so that the plan keeps [WorkspaceLayout.MIN_PLAN_DP].
 */
internal fun wideChatWidth(kind: WorkspaceLayout.Kind): Dp =
    if (kind == WorkspaceLayout.Kind.THREE_PANE) 360.dp else 320.dp

/** The rail's width, Material's. */
internal const val WideRailDp = 80

/**
 * Where the tree panel stands at [WorkspaceLayout.Kind.TWO_PANE], as a plain
 * holder so its rules are testable without a screen.
 *
 * The panel is a transient overlay, like the phone's plan sheet: it closes
 * when something is picked from it, when the rail place or the layout changes
 * under it, and on Back, which dismisses it before it leaves the workspace.
 * Where the tree is a column, or there are no panes, it is never shown, so a
 * stale flag can't surface it after a resize.
 */
class TreePanelState {
    var open by mutableStateOf(false)
        private set
    private var seenKind: WorkspaceLayout.Kind? = null
    private var seenDestination: WideDestination? = null

    /** Whether the panel is up at [kind]. */
    fun isShown(kind: WorkspaceLayout.Kind): Boolean = open && kind == WorkspaceLayout.Kind.TWO_PANE

    /** The rail's Plan place tapped while [selected] is up. True when the place should be selected. */
    fun planTapped(selected: WideDestination, kind: WorkspaceLayout.Kind): Boolean {
        if (selected == WideDestination.PLAN && kind == WorkspaceLayout.Kind.TWO_PANE) {
            open = !open
            return false
        }
        open = false
        return true
    }

    /** Something was picked from the panel (a row, a level, a terminal): it closes first, as the phone's sheet does. */
    fun picked() {
        open = false
    }

    /** The rail place or the layout changed, from here or from outside (a deep link, a resize, a fold). */
    fun sync(kind: WorkspaceLayout.Kind, destination: WideDestination) {
        if (kind != seenKind || destination != seenDestination) open = false
        seenKind = kind
        seenDestination = destination
    }

    /** Back: true when it closed the panel, so it didn't leave the workspace. */
    fun back(kind: WorkspaceLayout.Kind): Boolean {
        if (!isShown(kind)) return false
        open = false
        return true
    }
}

/** What a jump to a terminal does beside the chat. */
enum class WideJump { FOCUS_CHAT, OPEN }

/**
 * A Discuss or a jump to [ref], with [orchestrator] up in the chat pane: the
 * orchestrator's own terminal is already beside the plan, so it takes focus
 * instead of being pushed as a full screen over it.
 */
fun wideJump(ref: com.farcooler.net.TerminalRef, orchestrator: com.farcooler.model.Terminal?): WideJump =
    if (orchestrator != null && ref.terminalId == orchestrator.id) WideJump.FOCUS_CHAT else WideJump.OPEN

/**
 * The wide workspace's frame: a navigation rail, and beside it Material's
 * list-detail scaffold with the plan (or the board) as the detail pane and
 * the orchestrator's chat as the extra pane, which is the supporting pane of
 * the design. At [WorkspaceLayout.Kind.THREE_PANE] the tree is the list pane;
 * at [WorkspaceLayout.Kind.TWO_PANE] it is a panel over the plan's leading
 * edge, opened by tapping the rail's Plan place while it is selected ([panel]).
 *
 * Panes are divided by the theme's separator, drawn as vertical rules. The
 * plan's pane is given at least [WorkspaceLayout.MIN_PLAN_DP] by a width
 * constraint of its own, not by the scaffold's preferred widths.
 *
 * For TalkBack: the scrim is a labeled button, the plan behind an open panel
 * is hidden from it, focus moves into the panel on open and back to the rail's
 * Plan place on close, and that place says whether the tree is open.
 *
 * What goes in each pane is [WideDestination.contents]; [content] draws it.
 * The frame knows nothing about connections, so the tests and the captures
 * can drive it with the app's own views. [chatModifier] lets the screen focus
 * the chat pane.
 */
@Composable
fun WideWorkspaceFrame(
    kind: WorkspaceLayout.Kind,
    destination: WideDestination,
    panel: TreePanelState,
    onSelect: (WideDestination) -> Unit,
    onBack: () -> Unit,
    onNeedsYou: () -> Unit,
    topBar: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    chatModifier: Modifier = Modifier,
    content: @Composable (WideContent) -> Unit,
) {
    val shown = panel.isShown(kind)
    val contents = destination.contents(kind, shown)
    val columns = kind == WorkspaceLayout.Kind.THREE_PANE
    val panelFocus = remember { FocusRequester() }
    val planFocus = remember { FocusRequester() }
    // Back dismisses the panel before it leaves the workspace.
    BackHandler(enabled = shown) { panel.back(kind) }
    // Focus into the panel as it opens, and back to the rail's Plan place as it closes.
    var wasShown by remember { mutableStateOf(false) }
    LaunchedEffect(shown) {
        if (shown) panelFocus.requestFocus() else if (wasShown) planFocus.requestFocus()
        wasShown = shown
    }
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
                onClick = { if (panel.planTapped(destination, kind)) onSelect(WideDestination.PLAN) },
                icon = { Icon(Icons.Outlined.AccountTree, contentDescription = null) },
                label = { Text(WideDestination.PLAN.title) },
                modifier = Modifier.focusRequester(planFocus).testTag("wide-rail-plan").semantics {
                    if (!columns && destination == WideDestination.PLAN) stateDescription = if (shown) "Tree open" else "Tree closed"
                },
            )
            NavigationRailItem(
                selected = destination == WideDestination.BOARD,
                onClick = { if (panel.planTapped(WideDestination.BOARD, kind)) onSelect(WideDestination.BOARD) },
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
                    // Null-safe: a window shrinking from three columns to two can compose this pane's exit once more.
                    AnimatedPane(Modifier.preferredWidth(WideListWidth)) {
                        val tree = contents[WidePane.LIST]
                        if (columns && tree != null) {
                            Row(Modifier.fillMaxSize().testTag(WidePane.LIST.tag)) {
                                Box(Modifier.weight(1f).fillMaxHeight()) { content(tree) }
                                VerticalSeparator()
                            }
                        }
                    }
                },
                detailPane = {
                    AnimatedPane {
                        Row(Modifier.fillMaxSize()) {
                            Box(Modifier.weight(1f).fillMaxHeight().testTag(WidePane.MAIN.tag)) {
                                // The plan is never drawn narrower than its minimum, whatever the scaffold allots.
                                // Hidden from TalkBack while the panel is over it.
                                val main = contents[WidePane.MAIN]
                                Box(
                                    Modifier.fillMaxSize().widthIn(min = WorkspaceLayout.MIN_PLAN_DP.dp)
                                        .then(if (shown) Modifier.clearAndSetSemantics {} else Modifier),
                                ) { if (main != null) content(main) }
                                // The tree over the plan's leading edge, with a scrim that closes it.
                                val tree = contents[WidePane.LIST]
                                if (!columns && tree != null) {
                                    Box(
                                        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.scrim.copy(alpha = 0.32f))
                                            .clickable(onClickLabel = "Close tree", role = Role.Button, onClick = { panel.back(kind) })
                                            .testTag("wide-tree-scrim"),
                                    )
                                    Surface(
                                        modifier = Modifier.width(WideListWidth).fillMaxHeight().testTag(WidePane.LIST.tag)
                                            .semantics { paneTitle = "Tree" }
                                            .focusRequester(panelFocus).focusGroup(),
                                        tonalElevation = 3.dp,
                                        shadowElevation = 6.dp,
                                    ) {
                                        Row(Modifier.fillMaxSize()) {
                                            Box(Modifier.weight(1f).fillMaxHeight()) { content(tree) }
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
                    AnimatedPane(Modifier.preferredWidth(wideChatWidth(kind))) {
                        Box(Modifier.fillMaxSize().then(chatModifier).testTag(WidePane.SUPPORTING.tag)) {
                            contents[WidePane.SUPPORTING]?.let { content(it) }
                        }
                    }
                },
            )
        }
    }
}
