package com.farcooler.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.farcooler.account.Account
import com.farcooler.account.PushRegistration
import com.farcooler.data.Runner
import com.farcooler.data.RunnerStore
import com.farcooler.data.Identity
import com.farcooler.data.Settings
import com.farcooler.net.Connection
import com.farcooler.net.FleetRepository
import com.farcooler.net.Reachability
import com.farcooler.net.TerminalRef
import com.farcooler.notify.Notifier
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** Where the app is. */
sealed interface Route {
    /** No runners yet, so there is nothing else to show. */
    data object Onboarding : Route

    /** The worktree list — what a runner with nothing running shows. */
    data object Fleet : Route

    data class Terminal(val ref: TerminalRef) : Route

    data object Settings : Route

    data object Authorize : Route

    /**
     * This device showing a code, to be added by one that is already trusted.
     *
     * The short road out of [Authorize], which stays: pasting a key is what
     * works with no trusted device to scan with.
     */
    data object Join : Route

    /** The other side of the ceremony: granting runners to a device being added. */
    data object AddDevice : Route

    /** Everything this account has registered, and how to revoke it. */
    data object Devices : Route

    /**
     * One runner's own config.toml.
     *
     * Carries the host id rather than the connection, so a route survives the
     * connection being replaced underneath it — a reconnect builds a new
     * `Connection` and a route holding the old one would edit a dead session.
     */
    data class RunnerSettings(val hostId: String) : Route
}

/**
 * Everything the app is, in one place a composable can observe.
 *
 * A single model rather than one per screen. The alternative — a view model per
 * route — would mean the fleet is re-fetched every time someone opens a
 * terminal and re-connected every time they come back, on a product whose whole
 * claim is that every runner is already connected.
 */
class AppModel(application: Application) : AndroidViewModel(application) {
    val hosts = RunnerStore(application)
    val settings = Settings(application)
    val notifier = Notifier(application, settings)
    val account = Account(application)
    val push = PushRegistration(application, account)

    val fleet = FleetRepository(hosts, settings, viewModelScope)

    /**
     * The network coming back, which no backoff timer can predict.
     *
     * Held here rather than inside [FleetRepository] because it needs a
     * `Context`, and the repository deliberately has none — it is given the
     * stores it talks to, not the framework. Declared before `init`, because
     * that is where it is started and Kotlin initialises in source order.
     */
    private val reachability = Reachability(application) { fleet.reconnectAll() }

    private val _route = MutableStateFlow<Route>(Route.Fleet)
    val route: StateFlow<Route> = _route.asStateFlow()

    private val backStack = ArrayDeque<Route>()

    /**
     * Whether the app has decided where to open yet.
     *
     * Decided once per launch and then left alone. Recomputing it on every poll
     * would mean a terminal finishing its work while this screen is open — an
     * ordinary thing to happen while someone is reading it — yanks them onto a
     * different pane mid-read.
     */
    private val _landed = MutableStateFlow(false)
    val landed: StateFlow<Boolean> = _landed.asStateFlow()

    init {
        Identity.initialize(application)
        com.farcooler.data.Themes.initialize(application)
        notifier.createChannels()

        // Generate the device key at launch rather than the first time
        // something asks for it. It used to appear only when the authorise
        // screen was opened on iOS, which put a several-hundred-millisecond
        // keygen behind a tap and, worse, meant a runner could be added and
        // connected to before this device had an identity to offer.
        viewModelScope.launch { Identity.publicKey }

        fleet.onFleet = { host, snapshot ->
            for (workspace in snapshot.workspaces) {
                for (terminal in workspace.terminals) {
                    notifier.report(terminal, workspace.task, host.displayLabel)
                }
            }
        }

        account.afterSignIn = { viewModelScope.launch { push.sendIfPossible() } }
        push.attach(viewModelScope)

        reachability.start()
    }

    /**
     * Open onto TERMINALS, not onto a list of runners.
     *
     * Called once the first fleet has arrived. A list of runners is
     * onboarding, and onboarding is not a home screen — so the runner list
     * appears exactly when it is the thing to do, which is when there are no
     * runners.
     */
    fun landIfNeeded() {
        // A notification tap outranks the landing rule: someone who tapped
        // "claude needs you" asked for that pane by name.
        resolvePendingTerminal()
        if (_landed.value) return
        if (hosts.hosts.value.isEmpty()) {
            _landed.value = true
            _route.value = Route.Onboarding
            return
        }
        // Only once at least one runner has answered, or a slow SSH handshake
        // would land on the empty worktree list and stay there.
        val anySettled = fleet.active.value.any { it.phase.value !is Connection.Phase.Connecting }
        if (!anySettled) return

        _landed.value = true
        val target = fleet.landing()
        _route.value = if (target != null) Route.Terminal(target) else Route.Fleet
    }

    fun open(ref: TerminalRef) {
        if (_route.value is Route.Terminal) {
            // Switching terminals is not navigation: the terminal screen points
            // itself at a different pane without leaving, so the back stack
            // must not grow a frame per tap.
            _route.value = Route.Terminal(ref)
            return
        }
        navigate(Route.Terminal(ref))
    }

    /**
     * Open the terminal a notification was about.
     *
     * By id alone, because that is all a notification carries and all it can
     * carry: it may have been posted by the messaging service in a process that
     * had no fleet at all. The runner and worktree are looked up from whatever
     * has since connected, and a tap that arrives before the fleet does simply
     * lands on the list — which is the honest answer, not a guess.
     */
    fun openByTerminalId(terminalId: String) {
        pendingTerminal = terminalId
        resolvePendingTerminal()
    }

    private var pendingTerminal: String? = null

    private fun resolvePendingTerminal() {
        val wanted = pendingTerminal ?: return
        val entry = fleet.entries.value.firstOrNull { entry ->
            entry.workspace.terminals.any { it.id == wanted }
        } ?: return
        pendingTerminal = null
        _landed.value = true
        _route.value = Route.Terminal(TerminalRef(entry.host.id, entry.workspace.id, wanted))
    }

    fun navigate(route: Route) {
        backStack.addLast(_route.value)
        _route.value = route
    }

    /** True if there was somewhere to go back to. */
    fun back(): Boolean {
        val previous = backStack.removeLastOrNull() ?: return false
        _route.value = previous
        return true
    }

    fun showFleet() {
        _route.value = Route.Fleet
        backStack.clear()
    }

    /** A runner was added from onboarding, so leave onboarding. */
    fun addHost(host: Runner) {
        hosts.add(host)
        backStack.clear()
        _route.value = Route.Fleet
        _landed.value = false
    }

    fun removeHost(host: Runner) {
        hosts.remove(host)
        if (hosts.hosts.value.isEmpty()) {
            backStack.clear()
            _route.value = Route.Onboarding
        } else if (_route.value is Route.Terminal) {
            showFleet()
        }
    }

    fun setForeground(foreground: Boolean) {
        fleet.setForeground(foreground)
        notifier.isForeground = foreground
    }

    override fun onCleared() {
        super.onCleared()
        reachability.stop()
        // The scope is cancelled either way, but a native handle and an SSH
        // session are not the garbage collector's to reclaim.
        kotlinx.coroutines.runBlocking { fleet.close() }
    }
}
