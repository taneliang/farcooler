package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.isTraversalGroup
import androidx.compose.ui.semantics.semantics
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.net.NativePaneModel
import com.farcooler.net.NativePanes
import com.farcooler.net.PaneViewMemory

/**
 * One claude pane's conversation view, as its terminal pane sees it (ov-374):
 * the model where the runner offers the view, and the scroll position the view
 * keeps across the switch to the terminal and back.
 */
class NativePane(
    /** The pane's model, only while the conversation is offered here. */
    val model: NativePaneModel?,
    val listState: LazyListState,
) {
    /** The conversation covers the terminal now. */
    val covered: Boolean get() = model?.showing == true

    /** Whether the switch is drawn: offered, and the runner hasn't said it doesn't serve this pane. */
    val switchable: Boolean get() = model != null && !model.unavailable

    /** The switch's tap. */
    fun toggle() {
        model?.switchTo(!covered)
    }
}

/**
 * The conversation side of a terminal pane, wired: resolves the model, runs its
 * follow while it is due, and lets it go when it isn't.
 *
 * - [offered] is [com.farcooler.model.AgentConversation.offered]: the runner
 *   serves it, and the pane is a running claude or codex in a terminal. When it stops
 *   being true (the setting went off, claude exited) the model lets go and starts
 *   afresh when it's offered again.
 * - [live] is the pane being on screen with the app in front. The follow runs
 *   only then, and only while the conversation shows.
 * - Leaving composition stops the follow: the model lives in [panes], which
 *   outlives this pane, and a follow left running would hold a call on the runner
 *   for a pane nobody has.
 */
@Composable
fun rememberNativePane(
    terminalId: String,
    /** Claude, or codex where the runner says so ([com.farcooler.model.AgentConversation.isAgentInATerminal]). */
    agentInTerminal: Boolean,
    /** The pane's preset, which names the agent for its words and its keys. */
    preset: String,
    offered: Boolean,
    live: Boolean,
    /** What the runner's hello offers the composer: [AgentConversation.rich] and [AgentConversation.interrupts]. */
    rich: Boolean,
    interrupts: Boolean,
    panes: NativePanes,
    /** Bring here: [AgentConversation.bring] (ov-369). */
    bring: Boolean = false,
    memory: PaneViewMemory,
    /**
     * The conversation just covered the terminal, by the switch or by claude
     * starting in the pane: the terminal gives up the keyboard, so keys typed
     * there don't go to a terminal nobody can see.
     */
    onCovered: () -> Unit,
): NativePane {
    // Held by the connection, not by this composable, so the draft and rows
    // outlive the pane being evicted from the deck.
    val candidate = remember(terminalId, agentInTerminal, panes) {
        if (agentInTerminal) panes.model(terminalId, memory) else null
    }
    LaunchedEffect(candidate, preset) { candidate?.preset = preset }
    LaunchedEffect(candidate, offered, live) {
        candidate?.sync(offered, live)
    }
    LaunchedEffect(candidate, rich, interrupts, bring) {
        candidate?.offer(rich, interrupts, bring)
    }
    DisposableEffect(candidate) {
        onDispose { candidate?.removed() }
    }
    // Only the phase, so a streamed row doesn't redraw the pane.
    val phase = candidate?.store?.phase?.collectAsStateWithLifecycle()?.value
    LaunchedEffect(phase) { if (offered) candidate?.phaseChanged() }
    val pane = NativePane(candidate?.takeIf { offered }, rememberLazyListState())
    val covered = pane.covered
    LaunchedEffect(covered) { if (covered) onCovered() }
    return pane
}

/**
 * The pane's body: the terminal first and always (a switch never respawns it,
 * restarts its stream or loses its grid), and over it the conversation where
 * that shows.
 *
 * The covered terminal is out of reach of TalkBack and under the conversation's
 * touches, which it takes: a tap that fell through would raise the covered
 * terminal's keyboard, and keys typed there would go to a terminal nobody can
 * see.
 */
@Composable
fun NativeLayer(
    pane: NativePane,
    /** On a tab that draws no bar of its own, the switch floats over the pane. */
    floatingSwitch: Boolean,
    modifier: Modifier = Modifier,
    /** Why the conversation isn't offered, where the switch is dimmed and says so (ov-443). */
    unavailable: com.farcooler.model.AgentConversation.Unavailable? = null,
    terminal: @Composable () -> Unit,
) {
    Box(modifier.fillMaxSize()) {
        Box(Modifier.fillMaxSize().then(if (pane.covered) Modifier.clearAndSetSemantics {} else Modifier)) { terminal() }
        val model = pane.model
        if (model != null && pane.covered) {
            NativeAgentView(
                model = model,
                listState = pane.listState,
                showTerminal = { model.switchTo(false) },
                modifier = Modifier
                    // One group for a screen reader, ahead of the terminal under it.
                    .semantics { isTraversalGroup = true }
                    .background(MaterialTheme.colorScheme.background)
                    .pointerInput(Unit) { detectTapGestures { } },
            )
        }
        if (floatingSwitch && pane.switchable) {
            NativeSwitchButton(
                showing = pane.covered,
                onClick = { pane.toggle() },
                modifier = Modifier.align(Alignment.TopEnd),
            )
        } else if (floatingSwitch && unavailable != null) {
            NativeUnavailableButton(unavailable, Modifier.align(Alignment.TopEnd))
        }
    }
}
