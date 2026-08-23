package com.farcooler.ui

import androidx.activity.BackEventCompat
import androidx.activity.compose.BackHandler
import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.Text
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.net.Connection
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/**
 * The app, and the one decision it makes before anything else: where to open.
 *
 * Onto the FRONT DOOR — what needs a person, on every runner at once. It used
 * to open onto a terminal, chosen by a fleet-wide landing rule, and that is the
 * right answer to one of the four situations in `docs/jobs-to-be-done.md` and
 * the wrong answer to three. See [NeedsYouScreen] and [AppModel.landIfNeeded].
 *
 * A list of runners is still onboarding, and onboarding is still not a home
 * screen — so it appears exactly when it is the thing to do, which is when
 * there are no runners.
 *
 * ## Two layers, not one screen at a time
 *
 * The workspace — or the fleet list, or onboarding — is always composed, and
 * every pushed screen is drawn OVER it. That used to be a `when` with a
 * `return` per route, which meant opening settings took `TerminalScreen` out of
 * the composition entirely: the `DisposableEffect` disposed the session, the
 * second SSH channel behind it closed, the transcript's scroll went, and coming
 * back rebuilt all of it. It also meant a back gesture had nothing to preview,
 * because there was nothing behind the screen being dismissed.
 *
 * ## The drawer
 *
 * The fleet lives in a navigation drawer rather than behind a button, which is
 * where the Mac's sidebar and the phone's "switch terminal" sheet both end up
 * on this platform. It is the one place workspaces, runners, settings and
 * "start something new" all belong together, and an edge swipe reaches it
 * without a target to hit — which matters at 3am, one-handed, checking whether
 * the other agent is still blocked.
 *
 * The tab strip along the bottom of a terminal stays as well, and is not a
 * second copy of the drawer: it switches between panes with one tap and no
 * surface in the way, which is the thing you do constantly. The drawer is for
 * everything you do occasionally.
 */
@Composable
fun RootScreen(model: AppModel) {
    val stack by model.stack.collectAsStateWithLifecycle()
    val hosts by model.hosts.hosts.collectAsStateWithLifecycle()
    val entries by model.fleet.entries.collectAsStateWithLifecycle()
    val connections by model.fleet.active.collectAsStateWithLifecycle()

    val drawer = rememberDrawerState(DrawerValue.Closed)
    val scope = rememberCoroutineScope()

    // Decided once a runner has answered, then left alone. This is also where
    // a restored stack is checked against the fleet that just arrived — see
    // `AppModel.settle`.
    LaunchedEffect(entries, connections, hosts) { model.landIfNeeded() }

    val ground = stack.lastOrNull { !it.isOverlay } ?: Backstack.ROOT
    val overlays = stack.takeLastWhile { it.isOverlay }

    Box(Modifier.fillMaxSize()) {
        if (ground is Route.Onboarding || hosts.isEmpty()) {
            OnboardingScreen(
                onAdd = { model.addHost(it) },
                onAuthorize = { model.navigate(Route.Authorize) },
                onSettings = { model.navigate(Route.Settings) },
                showBack = ground !is Route.Onboarding || hosts.isNotEmpty(),
                onBack = { model.back() },
            )
        } else {
            // Back closes the drawer before it dismisses anything, and this is
            // deliberately NOT predictive. There is nothing behind a drawer to
            // preview — it is already drawn over the screen it returns you to —
            // and driving its offset from the gesture would mean reimplementing
            // `ModalNavigationDrawer`'s own animation to say the same thing.
            //
            // Registered before the overlays below, so an overlay's handler
            // wins when both are enabled: back handlers are dispatched
            // most-recently-added first.
            BackHandler(enabled = overlays.isEmpty() && drawer.isOpen) {
                scope.launch { drawer.close() }
            }

            // Back out of a pushed GROUND screen — a workspace, or the fleet
            // list — to whatever is under it, which is now always something,
            // because the front door is the root and a terminal is pushed onto
            // it rather than replacing it.
            //
            // The plain handler and not the predictive one, deliberately.
            // Predictive back needs the screen underneath to be composed, and
            // composing two ground screens at once would mean either giving up
            // the drawer's edge swipe on the screen it is most used from — the
            // gesture and the back gesture want the same edge — or mounting a
            // second terminal behind the first. Neither is worth an animation.
            // See [Route.isOverlay].
            BackHandler(enabled = overlays.isEmpty() && !drawer.isOpen && stack.size > 1) {
                model.back()
            }

            ModalNavigationDrawer(
                drawerState = drawer,
                // A drawer swipe would fight a back swipe from the same edge
                // while a screen is over it, and the back swipe is the one
                // that has somewhere to go.
                gesturesEnabled = overlays.isEmpty(),
                drawerContent = {
                    FleetDrawer(
                        model = model,
                        onSelect = { ref ->
                            model.open(ref)
                            scope.launch { drawer.close() }
                        },
                        onSettings = {
                            scope.launch { drawer.close() }
                            model.navigate(Route.Settings)
                        },
                        onAuthorize = {
                            scope.launch { drawer.close() }
                            model.navigate(Route.Authorize)
                        },
                    )
                },
            ) {
                Ground(model, ground, visible = overlays.isEmpty()) {
                    scope.launch { drawer.open() }
                }
            }
        }

        // Every pushed screen, bottom to top, each opaque. Only the top one
        // listens for the gesture; the one beneath is what the gesture reveals,
        // which is why they are all composed rather than only the last.
        //
        // All of them are wrapped, including the ones that cannot be swiped
        // yet. Wrapping only the top would mean popping a screen changes the
        // SHAPE of the composition beneath it — a branch on a condition is a
        // different subtree either side of it — and that discards the state of
        // the screen being returned to, which is the exact loss this layering
        // exists to prevent.
        overlays.forEachIndexed { index, route ->
            key(index, route) {
                DismissibleOverlay(
                    enabled = index == overlays.lastIndex,
                    onDismiss = { model.back() },
                ) {
                    OverlayScreen(model, route, connections)
                }
            }
        }
    }
}

/**
 * Whatever is underneath everything: the front door, a workspace, the workspace
 * list, or nothing yet.
 *
 * Exactly one of them is composed at a time, which is what separates a ground
 * route from an overlay. Back out of a pushed one is the plain handler above;
 * see [Route.isOverlay] for why the terminal did not become an overlay when it
 * started being pushed rather than replacing.
 *
 * Keyed on the RUNNER, and on nothing that can change when a chip is tapped.
 * That is the Compose form of the rule iOS wrote down in `09b1e1f`: which pane
 * is focused must not be able to restructure this subtree, because
 * restructuring it discards the terminal session, the transcript's scroll and
 * the half-typed message in the composer. The focus lives beside the stack in
 * `AppModel.focus`, and [Route.Terminal] deliberately has no room for it.
 *
 * The runner and not the workspace, matching the `remember(ref.hostId)` that
 * `TerminalScreen` already builds its session with: moving between workspaces
 * on one runner re-points the existing session rather than closing an SSH
 * channel and opening another. That one session is re-pointed at all is F-3 in
 * the parity inventory and is the pane-lifetime work, not this — what belongs
 * here is only that the key cannot contain a terminal id.
 */
@Composable
private fun Ground(model: AppModel, route: Route, visible: Boolean, onOpenDrawer: () -> Unit) {
    // Subscribed to rather than read. `paneOf` resolves against plain values —
    // the focus map and whichever fleet last arrived — so something has to make
    // this composable run again when they change, or a chip tap would move
    // nothing and an agent exiting would leave a dead pane on screen.
    // `TerminalScreen` subscribes to its connection's fleet the same way and
    // for the same reason.
    model.focus.collectAsStateWithLifecycle()
    model.fleet.entries.collectAsStateWithLifecycle()

    Box(Modifier.fillMaxSize()) {
        when (route) {
            is Route.Terminal -> key(route.hostId) {
                val ref = model.paneOf(route)
                if (ref == null) {
                    // Only reachable before the runner has answered: the moment
                    // it does and the workspace is empty, `settle` takes the
                    // route off the stack in the same turn.
                    EmptyWorkspace()
                } else {
                    TerminalScreen(
                        model = model,
                        ref = ref,
                        visible = visible,
                        onOpenDrawer = onOpenDrawer,
                    )
                }
            }

            is Route.Fleet -> FleetScreen(
                model = model,
                onSelect = { model.open(it) },
                onOpenDrawer = onOpenDrawer,
                onBack = { model.back() },
            )

            // The root, and the fallback for anything that has no ground of its
            // own. An overlay route reaching here would be a bug in
            // `Route.isOverlay` rather than something to draw; the front door
            // is what `Backstack.ROOT` degrades to, so it is what this degrades
            // to as well.
            else -> NeedsYouScreen(
                model = model,
                onSelect = { model.open(it) },
                onOpenWorkspace = { host, workspace -> model.openWorkspace(host, workspace) },
                onOpenWorkspaces = { model.navigate(Route.Fleet) },
                onOpenDrawer = onOpenDrawer,
            )
        }
    }
}

@Composable
private fun EmptyWorkspace() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Text("Waiting for that runner.")
    }
}

/** One pushed screen. Everything here is drawn over the ground, never instead of it. */
@Composable
private fun OverlayScreen(model: AppModel, route: Route, connections: List<Connection>) {
    when (route) {
        is Route.Settings -> SettingsScreen(
            model,
            onOpenRunnerSettings = { model.navigate(Route.RunnerSettings(it.host.id)) },
            onBack = { model.back() },
        )

        is Route.RunnerSettings -> {
            // Looked up fresh rather than carried: a reconnect replaces the
            // Connection, and a screen holding the old one would be editing a
            // session that no longer exists.
            val live = connections.firstOrNull { it.host.id == route.hostId }
            if (live == null) {
                model.back()
            } else {
                RunnerSettingsScreen(live, onBack = { model.back() })
            }
        }

        is Route.Authorize -> AuthorizeScreen(
            onJoin = { model.navigate(Route.Join) },
            onBack = { model.back() },
        )

        is Route.Join -> JoinScreen(model, onBack = { model.back() })

        is Route.AddDevice -> AddDeviceScreen(model, onBack = { model.back() })

        is Route.Devices -> DevicesScreen(model, onBack = { model.back() })

        // The ground's routes never reach here; `Route.isOverlay` is the one
        // place that split is decided.
        is Route.Onboarding, is Route.NeedsYou, is Route.Fleet, is Route.Terminal -> Unit
    }
}

/**
 * A screen you can swipe away, with the swipe visible while you are making it.
 *
 * `build.gradle.kts` has named predictive back as a reason for `minSdk = 37`
 * since the module was written, and `AndroidManifest.xml` sets
 * `enableOnBackInvokedCallback`, but every screen used the plain [BackHandler]
 * — which opts a screen OUT of the preview. So the platform animated nothing,
 * the gesture snapped at the end of it, and there was no way to see what you
 * were about to land on or to change your mind halfway. The claim was three
 * lines of comment and one manifest attribute; this is the part that was
 * missing.
 *
 * The treatment is Material's: the screen being dismissed shrinks away from the
 * thumb, rounds its corners as it lifts, and slides a little the way the
 * gesture is going, uncovering the edge it started from. What it uncovers is
 * the real screen underneath, composed all along — see [RootScreen].
 *
 * Cancelling springs back rather than snapping, which is the whole reason the
 * settle runs in the composition's scope instead of the gesture's: the gesture
 * coroutine is finished by the time we know it was cancelled.
 *
 * [enabled] is false for every screen but the top one. It stays composed and
 * wrapped regardless, so that becoming the top one is a flag flipping rather
 * than the subtree being rebuilt around it.
 */
@Composable
private fun DismissibleOverlay(
    enabled: Boolean,
    onDismiss: () -> Unit,
    content: @Composable () -> Unit,
) {
    val progress = remember { Animatable(0f) }
    var edge by remember { mutableIntStateOf(BackEventCompat.EDGE_LEFT) }
    val scope = rememberCoroutineScope()

    PredictiveBackHandler(enabled = enabled) { events ->
        try {
            events.collect { event ->
                edge = event.swipeEdge
                progress.snapTo(event.progress)
            }
            onDismiss()
            progress.snapTo(0f)
        } catch (cancelled: CancellationException) {
            // Not rethrown on purpose. The gesture was abandoned, not the
            // coroutine that hosts it, and the only thing left to do is put
            // the screen back where it was.
            scope.launch { progress.animateTo(0f) }
        }
    }

    Box(
        Modifier
            .fillMaxSize()
            .graphicsLayer {
                val fraction = progress.value
                if (fraction == 0f) return@graphicsLayer
                val shrink = 1f - 0.1f * fraction
                scaleX = shrink
                scaleY = shrink
                // Away from the thumb: a swipe from the left edge pushes the
                // screen right, uncovering the side the gesture started on.
                val direction = if (edge == BackEventCompat.EDGE_LEFT) 1f else -1f
                translationX = direction * fraction * size.width * 0.06f
                transformOrigin = TransformOrigin(if (direction > 0f) 1f else 0f, 0.5f)
                shape = RoundedCornerShape((28f * fraction).dp)
                clip = true
            }
    ) {
        content()
    }
}
