package com.farcooler.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.core.TerminalPalette
import com.farcooler.model.InboxRow
import com.farcooler.model.Workspace
import com.farcooler.net.TerminalRef
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * One worktree: the agents working in it, its diff, and every tab you have
 * opened still mounted behind the one on screen.
 *
 * This was `TerminalScreen` — one terminal, with a tab strip of the whole
 * fleet. The owner's account of reviewing an agent's work is what re-scoped it:
 * reading a diff is only part of the job, and the larger part is seeing what the
 * agent said it did, deciding whether that was right, and replying to it. Those
 * live in one worktree and you move between them constantly, so they are tabs of
 * one screen rather than two destinations with a Back between them. The reply
 * channel is the agent's own composer, one chip away.
 *
 * ## The Compose shape, and why it is not SwiftUI's
 *
 * `WorkspaceView` mounts every visited pane in a `ZStack`, hides the inactive
 * ones with `.opacity(0)` and `.allowsHitTesting(false)`, and pins identity with
 * `.id(pane.id)`. Transliterating that would miss what Compose actually needs
 * and what it hands you for free, so this is three mechanisms rather than one:
 *
 * 1. **A `Box` of every mounted pane, each in its own `key(pane.id)`.** This is
 *    the part that matters and the part that has no cheaper substitute:
 *    composition is what holds a `remember`, and a `TerminalSession`, an
 *    `AgentStream` and a `LazyListState` are all `remember`. `key` is Compose's
 *    identity pin — the direct analogue of `.id(_:)` — and it is what stops the
 *    slot table from recycling one pane's state onto another when the mounted
 *    set changes.
 * 2. **A `SaveableStateHolder`, bucketed by the same [Pane.id].** Not a
 *    substitute for mounting — it restores `rememberSaveable` state only, so on
 *    its own every tab tap would still close an SSH channel and replay a whole
 *    screen. What it adds is the thing SwiftUI has no equivalent of: it KEEPS
 *    the saved state of a key that has left the composition, so a pane evicted
 *    by [PaneDeck.MOUNT_LIMIT] gives back its half-typed message when you return
 *    to it, and so does a pane that was mounted before the process was killed.
 *    It is also what makes a per-pane draft certain rather than a consequence of
 *    how `rememberSaveable` derives its key.
 * 3. **Hidden, not removed**, by `alpha(0f)` plus a pointer blocker plus
 *    `clearAndSetSemantics`. Compose has no `allowsHitTesting`, and z-order
 *    alone is not enough: a region the visible pane does not consume falls
 *    through to whatever is under it. See [mountedPane].
 *
 * What was NOT chosen, and why. A `HorizontalPager` looks like the native answer
 * and is not: it discards pages past `beyondViewportPageCount`, which is the
 * defect being fixed, and its swipe would fight the terminal canvas's own pan.
 * A `SaveableStateHolder` alone loses the stream. `movableContentOf` moves a
 * subtree between call sites, which is not the problem — nothing here moves.
 *
 * ## What mounting costs, and what pays for it
 *
 * A mounted pane holds its STATE. It deliberately does not hold its TRAFFIC:
 * `live` below is false for every tab but the one on screen, and false for all
 * of them while the app is backgrounded, and that flag is what starts and stops
 * the SSH stream, the agent poll and the tmux geometry assertion. Without that
 * split, a phone with three tabs open would hold three second channels while it
 * sat in a pocket — which is a bigger regression than the bug this fixes.
 *
 * ## The tab strip is still at the bottom
 *
 * iOS moved its strip to a floating overlay under the navigation bar. This one
 * stays in thumb reach, with the reason `cb13d31` gave: under the title bar it
 * would put the one control you use constantly at the far end of the screen from
 * the hand holding the phone. The SCOPE changed; the position did not.
 */
@Composable
fun WorkspaceScreen(
    model: AppModel,
    route: Route.Terminal,
    /**
     * Whether this screen is the thing being looked at, rather than composed
     * under a pushed screen.
     *
     * Distinct from a pane's own `live`, and the two answer different
     * questions. This one decides whether to claim the runner's attention
     * register — suppressing this pane's banners and marking a `done` agent
     * seen — which a screen under the settings sheet must not do. It
     * deliberately does NOT stop the stream: a trip into settings is short, the
     * terminal behind it is expected to still be live, and `df87410` kept the
     * ground composed precisely so that trip stopped costing an SSH channel.
     */
    onScreen: Boolean = true,
    onOpenDrawer: () -> Unit,
) {
    val connection = model.fleet.connection(route.hostId) ?: run {
        // The runner this workspace was on is gone — removed in settings, or
        // its connection torn down and rebuilt. Saying so beats a blank screen.
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Text("That runner is no longer connected.")
        }
        return
    }

    val scope = rememberCoroutineScope()
    val focus by model.focus.collectAsStateWithLifecycle()
    val entries by model.fleet.entries.collectAsStateWithLifecycle()
    val connections by model.fleet.active.collectAsStateWithLifecycle()
    connection.fleet.collectAsStateWithLifecycle()

    // Not `collectAsStateWithLifecycle`, deliberately. That one stops
    // collecting below STARTED, and the transition this drives — the app
    // leaving the foreground — is precisely a lifecycle transition. It happens
    // to be observable at `onPause`, where STARTED is still active, but
    // depending on that ordering to decide whether a phone in a pocket holds an
    // SSH channel open is a bet with nothing to gain.
    val foreground by model.foreground.collectAsState()

    val entry = entries.firstOrNull {
        it.host.id == route.hostId && it.workspace.id == route.workspaceId
    }
    val workspace: Workspace? = entry?.workspace
    val counts: InboxRow? = entry?.counts

    // Where the app has been TOLD to be, which is not the same as the rule's
    // guess. Only the focus map moves this screen: it changes when somebody
    // taps a chip, a fleet row, a drawer entry or a notification, and it does
    // not change because an agent somewhere else in the worktree finished. The
    // rule gets exactly one turn — the deck's opening tab — for the reason iOS
    // states on `WorkspaceView.select`: a rule that keeps running would move a
    // screen somebody is reading.
    val requested = Backstack.resolve(
        focus[Backstack.key(route.hostId, route.workspaceId)]?.pane,
        workspace?.terminals.orEmpty(),
    )

    // The rule's one turn: the tab this workspace opens on. After this the deck
    // is the authority and nothing recomputes it.
    //
    // **Built once and then never null again**, which is not defensiveness. A
    // reconnect can empty a runner's fleet for a poll, and `paneOf` answers null
    // for a workspace with no panes — so recomputing this on every composition
    // and bailing out on null would take the whole subtree out of the
    // composition, disposing every mounted session, on a blip. That is the
    // hazard `PaneDeck.prune` refuses one level down, and refusing it there is
    // worth nothing if this level hands it back. It cost only one session before
    // this phase, which is why the shape survived until now.
    var deck by remember { mutableStateOf(model.paneOf(route)?.let { PaneDeck.opening(it) }) }
    // The cold case: a workspace opened from a restored stack, before its runner
    // has finished its handshake. `remember`'s initializer already covers the
    // ordinary case, so this never costs a frame of "Waiting" on a fleet that is
    // already here.
    LaunchedEffect(requested, workspace) {
        if (deck == null) model.paneOf(route)?.let { deck = PaneDeck.opening(it) }
    }
    LaunchedEffect(requested) { requested?.let { pane -> deck = deck?.select(pane) } }
    LaunchedEffect(workspace?.terminals) {
        val terminals = workspace?.terminals ?: return@LaunchedEffect
        deck = deck?.prune(terminals)
    }

    val holder = rememberSaveableStateHolder()
    // Saved state is kept for an EVICTED pane and dropped for a PRUNED one, and
    // the difference is the whole rule. Eviction is this phone deciding it will
    // not hold four emulators; the tab still exists and you are likely to come
    // back to it, so the message you were half way through writing waits for
    // you. A pruned pane is one the runner no longer has, and a draft addressed
    // to an agent that is gone is a line in the saved state that can never be
    // spent again.
    val everMounted = remember { mutableSetOf<String>() }
    LaunchedEffect(deck?.mounted, workspace?.terminals) {
        val mounted = deck?.mounted ?: return@LaunchedEffect
        mounted.forEach { everMounted += it.id }
        // Only against a workspace the runner has actually described. A null or
        // empty terminal list is a reconnect or a runner mid-restart, and
        // acting on it would delete every draft on the evidence of a poll that
        // answered nothing — the same trap `PaneDeck.prune` refuses.
        val terminals = workspace?.terminals?.takeIf { it.isNotEmpty() }
            ?: return@LaunchedEffect
        val alive = terminals.map { Pane.Terminal(it.id).id }.toSet() + Pane.CHANGES_ID
        // Minus whatever is on screen right now. The prune above and this run
        // in the same frame, so there is a moment where the deck still holds a
        // pane the fleet has dropped — and `removeState` on a key that is still
        // mounted takes its live registry out from under it.
        val gone = everMounted - alive - mounted.map { it.id }.toSet()
        gone.forEach { holder.removeState(it) }
        everMounted -= gone
    }

    // The keyboard belongs to whichever pane is on screen, and a real Android
    // `View` holds the terminal's — see `TerminalInputView`. Compose's focus
    // manager cannot reach into that, so each pane dismisses its own on the way
    // out (`TerminalPane`, on `live`); this clears the Compose-side focus, which
    // is the agent composer's. Without both, typing after a tab tap goes to a
    // pane nobody can see.
    val focusManager = LocalFocusManager.current
    LaunchedEffect(deck?.current) { focusManager.clearFocus(force = true) }

    // Which pane the runner should believe is being read.
    //
    // Null on the Changes tab, and that is the honest answer rather than a gap:
    // no pane is on screen, so no pane's notification should be suppressed and
    // no agent's finished turn should be marked seen. `Connection.markVisibleSeen`
    // reads exactly this and does nothing without an id.
    val reading = (deck?.current as? Pane.Terminal)?.terminalId
    DisposableEffect(reading, onScreen) {
        if (onScreen) {
            connection.visibleTerminal = reading
            model.notifier.visibleTerminal = reading
            scope.launch { connection.markVisibleSeen() }
        }
        onDispose {
            if (onScreen) {
                connection.visibleTerminal = null
                model.notifier.visibleTerminal = null
            }
        }
    }

    // Images on their way into a terminal. Owned HERE rather than per pane, so
    // a transfer keeps running — and keeps reporting — when you switch away
    // from the pane that started it. It was already keyed to nothing inside the
    // old single screen for the same reason; this is where "nothing" now lives.
    val pastes = remember { ImagePasteQueue() }
    val resolver = LocalContext.current.contentResolver
    val pickImage = rememberLauncherForActivityResult(
        ActivityResultContracts.PickVisualMedia()
    ) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        // No terminal to type a path into — only reachable if the tab moved
        // while the picker was up.
        val target = (deck?.current as? Pane.Terminal)?.terminalId
            ?: return@rememberLauncherForActivityResult
        scope.launch {
            val picked = withContext(Dispatchers.IO) { readPickedImage(resolver, uri) }
            if (picked == null) {
                pastes.reject("Far Cooler couldn’t read that image.")
                return@launch
            }
            pastes.send(
                picked.data,
                picked.name,
                picked.mime,
                picked.thumbnail,
                target,
                connection.core,
                scope,
            )
        }
    }

    val panes = deck
    if (panes == null) {
        // Only reachable before the runner has answered anything at all. The
        // moment it does and the workspace turns out to be empty,
        // `AppModel.settle` takes the route off the stack in the same turn.
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Text("Waiting for that runner.")
        }
        return
    }

    Column(
        Modifier
            .fillMaxSize()
            // The terminal's own ground, all the way under the tab strip. A
            // background that stopped at the pane would leave the band behind
            // the strip showing the theme's surface, which reads as a bar
            // across the bottom rather than as the terminal continuing.
            .background(Color(TerminalPalette.BACKGROUND))
            .imePadding()
    ) {
        Box(Modifier.weight(1f)) {
            for (pane in panes.mounted) {
                val showing = pane == panes.current
                key(pane.id) {
                    holder.SaveableStateProvider(pane.id) {
                        Box(Modifier.fillMaxSize().mountedPane(showing)) {
                            when (pane) {
                                is Pane.Terminal -> TerminalPane(
                                    model = model,
                                    ref = TerminalRef(
                                        route.hostId, route.workspaceId, pane.terminalId),
                                    connection = connection,
                                    workspace = workspace,
                                    showRunner = connections.size > 1,
                                    // Everything this pane costs the runner
                                    // follows this and nothing else.
                                    live = showing && foreground,
                                    onPickImage = {
                                        pickImage.launch(
                                            PickVisualMediaRequest(
                                                ActivityResultContracts.PickVisualMedia.ImageOnly
                                            )
                                        )
                                    },
                                    onOpenDrawer = onOpenDrawer,
                                )

                                is Pane.Changes -> ChangesTab(
                                    workspace = workspace,
                                    counts = counts,
                                    showRunner = connections.size > 1,
                                    runnerLabel = connection.host.displayLabel,
                                    onOpenDrawer = onOpenDrawer,
                                )
                            }
                        }
                    }
                }
            }

            // Over the panes, and gone the moment the path is typed. Nothing
            // about a transfer is ever written into the pane itself: the path is
            // the only thing that reaches the program.
            ImagePasteChips(pastes, Modifier.align(Alignment.BottomCenter))
        }

        // `choose`, not `open`: this strip is the ONE writer of the remembered
        // focus. A chip is a person saying where they want to be, which is
        // exactly what a fleet row and a tapped notification are not — see
        // [Focus]. And because the strip is scoped to this workspace, a chip
        // never moves the navigation stack at all: `choose` finds the workspace
        // it is already on and installs nothing.
        TerminalTabStrip(
            workspace = workspace,
            counts = counts,
            current = panes.current,
            onSelect = { model.choose(route.hostId, route.workspaceId, it) },
            modifier = Modifier.navigationBarsPadding(),
        )
    }
}

/**
 * Hidden, not removed.
 *
 * `alpha` keeps the pane in the hierarchy — which is what preserves its state —
 * while the rest of this stops a pane nobody can see from taking anything meant
 * for the one on top of it. Three separate leaks to close, because Compose has
 * no single `allowsHitTesting`:
 *
 * - **Touches.** `zIndex` puts the showing pane first in both draw order and
 *   hit-test order, but hit testing continues past a child that does not
 *   CONSUME the event — so an empty region of a chat would hand a tap to the
 *   terminal underneath it. Consuming in the `Initial` pass is the parent-first
 *   pass, so every gesture detector below sees an already-consumed change.
 * - **Focus.** `canFocus = false` stops the focus system from traversing into a
 *   hidden pane's composer. Focus already inside one is cleared by the screen,
 *   which is the only place that knows a tab changed.
 * - **Accessibility.** Without `clearAndSetSemantics`, TalkBack reads three
 *   transcripts stacked on top of each other, all of them invisible.
 */
private fun Modifier.mountedPane(showing: Boolean): Modifier =
    this
        .zIndex(if (showing) 1f else 0f)
        .alpha(if (showing) 1f else 0f)
        .then(if (showing) Modifier else HIDDEN_PANE)

private val HIDDEN_PANE: Modifier = Modifier
    .focusProperties { canFocus = false }
    .clearAndSetSemantics { }
    .pointerInput(Unit) {
        awaitPointerEventScope {
            while (true) {
                awaitPointerEvent(PointerEventPass.Initial).changes.forEach { it.consume() }
            }
        }
    }

/**
 * The worktree's diff, before there is a surface that can show one.
 *
 * **Phase 5 of the parity program builds the review, and this is not it.** What
 * this phase owed was a Changes CHIP that does something honest, and leaving it
 * dead was explicitly not an option — so the tab exists, holds its seat in the
 * deck and says what it actually knows: how much the worktree has changed, and
 * that reading it here is not built yet.
 *
 * That is more than a placeholder, because of where it sits. `Terminal.isChangesPane`
 * has been decoded since phase 1 with a note saying it had no caller and that a
 * host-side `changes` pane therefore fell past `isAgentPane` to the VT renderer
 * and was drawn as a grid of raw bytes — the defect the parity inventory found
 * on its way past `TerminalScreen.kt:334`. It has a caller now: [Pane] folds
 * such a pane into this tab from every direction — the remembered focus, the
 * landing rule, the strip, and the deck's prune — so the raw-bytes screen is no
 * longer reachable. Until phase 5 lands, THIS is what a `changes` pane shows,
 * and a sentence saying where the diff can be read is strictly better than a
 * screenful of escape sequences.
 *
 * When the review does land it replaces the body of this composable and nothing
 * else: the chip, the tab, the focus entry and the deck all already exist.
 */
@Composable
private fun ChangesTab(
    workspace: Workspace?,
    counts: InboxRow?,
    showRunner: Boolean,
    runnerLabel: String,
    onOpenDrawer: () -> Unit,
) {
    Column(Modifier.fillMaxSize()) {
        WorkspaceTopBar(
            workspace = workspace,
            fallbackTitle = "Changes",
            showRunner = showRunner,
            runnerLabel = runnerLabel,
            onOpenDrawer = onOpenDrawer,
        )
        Column(
            Modifier.fillMaxSize().padding(horizontal = 32.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text("Changes", style = MaterialTheme.typography.titleMedium)
            if (counts != null && counts.hasDiff) {
                Spacer(Modifier.height(8.dp))
                DiffCounts(counts)
            }
            Spacer(Modifier.height(10.dp))
            Text(
                changesTabMessage(counts),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
        }
    }
}

/**
 * What the Changes tab says, in three states that are genuinely different.
 *
 * A null [counts] is the absence of a fact and not the fact that there is
 * nothing — `model/NeedsYou.kt` draws the same line, and it matters more here,
 * where the alternative is telling somebody their branch is clean because their
 * runner has not finished its handshake.
 *
 * Pure, so the words can be read by a test rather than by a screenshot.
 */
internal fun changesTabMessage(counts: InboxRow?): String = when {
    counts == null ->
        "Waiting for this runner to say what’s changed here."
    !counts.hasDiff ->
        "Nothing has changed on this branch yet. When an agent writes something, " +
            "the counts show up on this chip."
    else ->
        "That’s everything this worktree has changed, committed or not. Reading the " +
            "diff on your phone isn’t built yet — it’s on your Mac for now, and the " +
            "agents that wrote it are one chip away."
}
