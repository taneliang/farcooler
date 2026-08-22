package com.farcooler.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch

/**
 * The app, and the one decision it makes before anything else: where to open.
 *
 * Onto TERMINALS, not onto a list of runners. A list of runners is
 * onboarding, and onboarding is not a home screen — so it appears exactly when
 * it is the thing to do, which is when there are no runners.
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
    val route by model.route.collectAsStateWithLifecycle()
    val hosts by model.hosts.hosts.collectAsStateWithLifecycle()
    val entries by model.fleet.entries.collectAsStateWithLifecycle()
    val connections by model.fleet.active.collectAsStateWithLifecycle()

    val drawer = rememberDrawerState(DrawerValue.Closed)
    val scope = rememberCoroutineScope()

    // Decided once a runner has answered, then left alone.
    LaunchedEffect(entries, connections, hosts) { model.landIfNeeded() }

    if (route is Route.Onboarding || hosts.isEmpty()) {
        OnboardingScreen(
            onAdd = { model.addHost(it) },
            onAuthorize = { model.navigate(Route.Authorize) },
            onSettings = { model.navigate(Route.Settings) },
            showBack = route !is Route.Onboarding || hosts.isNotEmpty(),
            onBack = { model.back() },
        )
        // Drawn over onboarding, because both are reachable with no runners at
        // all — being granted some is exactly what the ceremony is for.
        when (route) {
            is Route.Authorize -> AuthorizeScreen(
                onJoin = { model.navigate(Route.Join) },
                onBack = { model.back() },
            )

            is Route.Join -> JoinScreen(model, onBack = { model.back() })

            else -> Unit
        }
        return
    }

    // Everything but the terminal is a pushed screen, so back leaves it. From
    // the terminal itself, back closes the drawer if it is open and otherwise
    // does what back does on Android: leaves the app. A terminal is the home
    // screen, and a home screen with a back button that goes somewhere is a
    // home screen that is not one.
    BackHandler(enabled = drawer.isOpen) {
        scope.launch { drawer.close() }
    }

    when (val current = route) {
        is Route.Settings -> {
            BackHandler { model.back() }
            SettingsScreen(
                model,
                onOpenRunnerSettings = { model.navigate(Route.RunnerSettings(it.host.id)) },
                onBack = { model.back() },
            )
            return
        }

        is Route.RunnerSettings -> {
            BackHandler { model.back() }
            // Looked up fresh rather than carried: a reconnect replaces the
            // Connection, and a screen holding the old one would be editing a
            // session that no longer exists.
            val live = connections.firstOrNull { it.host.id == current.hostId }
            if (live == null) {
                model.back()
            } else {
                RunnerSettingsScreen(live, onBack = { model.back() })
            }
            return
        }

        is Route.Authorize -> {
            BackHandler { model.back() }
            AuthorizeScreen(
                onJoin = { model.navigate(Route.Join) },
                onBack = { model.back() },
            )
            return
        }

        is Route.Join -> {
            BackHandler { model.back() }
            JoinScreen(model, onBack = { model.back() })
            return
        }

        is Route.AddDevice -> {
            BackHandler { model.back() }
            AddDeviceScreen(model, onBack = { model.back() })
            return
        }

        is Route.Devices -> {
            BackHandler { model.back() }
            DevicesScreen(model, onBack = { model.back() })
            return
        }

        else -> Unit
    }

    ModalNavigationDrawer(
        drawerState = drawer,
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
        Box(Modifier.fillMaxSize()) {
            when (val current = route) {
                is Route.Terminal -> TerminalScreen(
                    model = model,
                    ref = current.ref,
                    onOpenDrawer = { scope.launch { drawer.open() } },
                )

                else -> FleetScreen(
                    model = model,
                    onSelect = { model.open(it) },
                    onOpenDrawer = { scope.launch { drawer.open() } },
                )
            }
        }
    }
}
