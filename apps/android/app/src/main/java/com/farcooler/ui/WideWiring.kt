package com.farcooler.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.movableContentOf
import androidx.compose.runtime.remember
import com.farcooler.model.Terminal
import com.farcooler.net.TerminalRef

/**
 * What `WorkspaceScreen` wires around the wide frame, as a plain holder so the
 * wiring is testable on the JVM (ov-353 R2-3): the tree panel, its sync with
 * the rail place and the layout, the close-on-pick wrapper for the tree's
 * navigation, and where a jump goes. The screen holds one per workspace and
 * calls these; nothing here is drawn.
 *
 * [focusChat] moves focus into the chat pane; it is the one thing the screen
 * supplies that needs a composition.
 */
class WideWiring(private val focusChat: () -> Unit) {
    val panel = TreePanelState()

    /** The layout or the rail place (from a tab, a rail tap or outside) may have changed. */
    fun sync(kind: WorkspaceLayout.Kind, tab: WorkspaceTab) = panel.sync(kind, WideDestination.of(tab))

    /** The tree's navigation, closing the panel before whatever a pick does, as the phone's sheet does. */
    fun treeNavigation(base: TreeNavigation): TreeNavigation = base.then { panel.picked() }

    /**
     * A Discuss or a jump to [ref]. In a wide layout the orchestrator's own
     * terminal is the chat beside the plan, so it takes focus; anything else is
     * [open]ed. Used by the plan and by the board, so both behave alike.
     */
    fun jump(ref: TerminalRef, orchestrator: Terminal?, open: (TerminalRef) -> Unit) {
        when (wideJump(ref, orchestrator)) {
            WideJump.FOCUS_CHAT -> focusChat()
            WideJump.OPEN -> open(ref)
        }
    }
}

/**
 * One block of UI that can be called from the phone's slot and the wide
 * frame's, keeping its state (a half-typed message, a scroll) when the window
 * crosses a breakpoint. The orchestrator's pane is built with it. Proved in
 * `MovablePaneSurvivalTest`, which folds and unfolds a real frame around it.
 */
@Composable
fun <T> rememberMovablePane(key: Any?, content: @Composable (T) -> Unit): @Composable (T) -> Unit =
    remember(key) { movableContentOf(content) }
