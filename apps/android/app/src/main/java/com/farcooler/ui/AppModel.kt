package com.farcooler.ui

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.viewModelScope
import com.farcooler.account.Account
import com.farcooler.account.PushRegistration
import com.farcooler.data.Runner
import com.farcooler.data.RunnerStore
import com.farcooler.data.Identity
import com.farcooler.data.Settings
import com.farcooler.model.Terminal
import com.farcooler.net.Connection
import com.farcooler.net.FleetRepository
import com.farcooler.net.Reachability
import com.farcooler.net.TerminalRef
import com.farcooler.notify.Notifier
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * Everything the app is, in one place a composable can observe.
 *
 * A single model rather than one per screen. The alternative — a view model per
 * route — would mean the fleet is re-fetched every time someone opens a
 * terminal and re-connected every time they come back, on a product whose whole
 * claim is that every runner is already connected.
 *
 * ## Surviving the process
 *
 * [SavedStateHandle], not `rememberSaveable`, is where navigation is written
 * down. A saveable in the composition would be scoped to the composable that
 * declared it, and where the app IS is not one screen's business — it is read
 * by `RootScreen` before any screen exists and written by a notification tap
 * that may arrive with no composition at all. `rememberSaveable` is the right
 * tool one level down, for a screen's own scroll and drafts, and is used there.
 *
 * `docs/jobs-to-be-done.md` F4 is the owner saying the phone has to survive
 * being put down every ninety seconds and that a review has to be resumable.
 * Android never noticed this was missing because `AndroidManifest.xml` handles
 * `orientation` itself, so rotation never recreates the activity — the only
 * rehearsal most apps get for the real thing.
 */
class AppModel(
    application: Application,
    private val saved: SavedStateHandle,
) : AndroidViewModel(application) {
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

    private val _stack = MutableStateFlow(listOf(Backstack.ROOT))

    /**
     * Every screen that is stacked up, root first, what is on screen last.
     *
     * Exposed whole rather than as a top and a hidden `ArrayDeque`, because
     * `RootScreen` needs to know both what to draw and whether there is
     * anything underneath it to preview during a back gesture.
     */
    val stack: StateFlow<List<Route>> = _stack.asStateFlow()

    private val _route = MutableStateFlow(_stack.value.last())

    /** What is on screen: the top of [stack], kept beside it so screens can observe just that. */
    val route: StateFlow<Route> = _route.asStateFlow()

    private val _focus = MutableStateFlow<Map<String, Focus>>(emptyMap())

    /**
     * Which pane each workspace is showing, keyed `runner/workspace`.
     *
     * **Beside the stack, never inside it.** See [Route.Terminal] for why, and
     * [Focus] for why only half of what lands here is written down.
     *
     * Keyed by runner AND workspace because this app is connected to every
     * runner at once: workspace ids are minted per daemon, so a memory keyed by
     * workspace alone would let one runner's worktree answer for another's.
     * iOS can key by workspace alone only because its equivalent lives on a
     * `Connection` that dies with the runner.
     */
    val focus: StateFlow<Map<String, Focus>> = _focus.asStateFlow()

    /**
     * Whether the app has decided where to open yet.
     *
     * Decided once per launch and then left alone. Recomputing it on every poll
     * would mean a terminal finishing its work while this screen is open — an
     * ordinary thing to happen while someone is reading it — yanks them onto a
     * different pane mid-read.
     *
     * Saved, because after a process death the app has already decided: the
     * stack that just came back IS the decision, and landing again would throw
     * it away in the one case restoration exists for. A restored `[Fleet]` is
     * indistinguishable from a cold start without this, and somebody who had
     * deliberately gone back to the workspace list would be dropped into a
     * terminal for it.
     */
    private val _landed = MutableStateFlow(saved.get<Boolean>(LANDED) ?: false)
    val landed: StateFlow<Boolean> = _landed.asStateFlow()

    init {
        Identity.initialize(application)
        com.farcooler.data.Themes.initialize(application)
        notifier.createChannels()

        // Restored before anything else runs, so the first composition already
        // has the right answer rather than flashing the root and correcting
        // itself. Nothing here is checked against a fleet yet — there is no
        // fleet at this point in a launch — which is what `settle` is for.
        Backstack.decodeStack(saved[STACK])?.let { install(it, persist = false) }
        _focus.value = Backstack.decodeFocus(saved[FOCUS])

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
        // Before landing, and before the pending-notification check, because
        // both of those read the stack: a route naming a workspace that is gone
        // has to be off the stack before anything decides whether the app has
        // somewhere to be.
        settle()

        // A notification tap outranks the landing rule: someone who tapped
        // "claude needs you" asked for that pane by name.
        resolvePendingTerminal()
        if (_landed.value) return
        if (hosts.hosts.value.isEmpty()) {
            land(Route.Onboarding)
            return
        }
        // Only once at least one runner has answered, or a slow SSH handshake
        // would land on the empty workspace list and stay there.
        val anySettled = fleet.active.value.any { it.phase.value !is Connection.Phase.Connecting }
        if (!anySettled) return

        val target = fleet.landing()
        if (target == null) {
            land(Backstack.ROOT)
            return
        }
        // The landing rule is the app choosing, not a person choosing, so it
        // points rather than records — the memory would otherwise be seeded
        // with its own guess and the rule would never run in this workspace
        // again.
        point(target)
        land(Route.Terminal(target.hostId, target.workspaceId))
    }

    private fun land(route: Route) {
        _landed.value = true
        saved[LANDED] = true
        install(listOf(route))
    }

    /**
     * Go to a pane somebody was SENT to — a fleet row, the drawer, a push.
     *
     * Replaces the workspace rather than pushing one, because the workspace IS
     * the home screen here: a stack of workspaces would give the terminal a
     * back button that goes to another terminal, and a home screen you can go
     * back out of is not one.
     */
    fun open(ref: TerminalRef) {
        point(ref)
        goTo(Route.Terminal(ref.hostId, ref.workspaceId))
    }

    /**
     * Go to a pane somebody CHOSE — the one writer of the remembered focus.
     *
     * Called from the tab strip and nowhere else. The strip spans the whole
     * fleet, so a chip may belong to another workspace on another runner; when
     * it does this is navigation as well, and when it does not **the stack is
     * not touched at all**. That is the property the whole shape exists for:
     * tapping a chip moves no navigation state, so nothing keyed on the route
     * has a reason to reset. See [Route.Terminal].
     *
     * Choosing the chip you are already on still records. Confirming the rule's
     * guess is a choice, and the next visit should not have to guess again.
     */
    fun choose(ref: TerminalRef) {
        rememberChoice(ref)
        goTo(Route.Terminal(ref.hostId, ref.workspaceId))
    }

    /**
     * Put a workspace on screen, whatever was on it before.
     *
     * A workspace REPLACES the stack rather than pushing onto it. `RootScreen`
     * has said since it was written that a terminal is the home screen and that
     * a home screen with a back button leading somewhere is not one — but the
     * code had drifted: opening a terminal from the workspace list pushed, so
     * the list accumulated underneath and only stayed invisible because no
     * screen on that branch had a back handler to pop it. Now the comment is
     * true. The workspace list is an edge swipe away in the drawer, which is
     * what it is for.
     *
     * Arriving at the workspace already underneath only closes what is over it,
     * so a push for a pane in the workspace you are already in costs no
     * navigation at all.
     */
    private fun goTo(target: Route.Terminal) {
        val base = _stack.value.dropLastWhile { it.isOverlay }
        install(if (base.lastOrNull() == target) base else listOf(target))
    }

    /**
     * Open the terminal a notification was about.
     *
     * By id alone, because that is all a notification carries and all it can
     * carry: it may have been posted by the messaging service in a process that
     * had no fleet at all. The runner and workspace are looked up from whatever
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
        saved[LANDED] = true
        // `open`, not `choose`: a 3am ping is not a preference about where this
        // workspace should open tomorrow.
        open(TerminalRef(entry.host.id, entry.workspace.id, wanted))
    }

    fun navigate(route: Route) {
        install(_stack.value + route)
    }

    /** True if there was somewhere to go back to. */
    fun back(): Boolean {
        if (_stack.value.size <= 1) return false
        install(_stack.value.dropLast(1))
        return true
    }

    fun showFleet() {
        install(listOf(Backstack.ROOT))
    }

    /** A runner was added from onboarding, so leave onboarding. */
    fun addHost(host: Runner) {
        hosts.add(host)
        install(listOf(Backstack.ROOT))
        _landed.value = false
        saved[LANDED] = false
    }

    fun removeHost(host: Runner) {
        hosts.remove(host)
        if (hosts.hosts.value.isEmpty()) {
            install(listOf(Route.Onboarding))
        } else {
            // Everything that named that runner stops naming anything, which
            // `settle` already knows how to handle — including the runner
            // settings screen for the runner being removed, which is where
            // this is usually called from.
            settle()
        }
    }

    /**
     * Which pane a workspace route is showing right now, or null while there is
     * nothing to show it with.
     *
     * Resolved fresh on every read rather than stored, for the reason
     * `TerminalScreen` looks its terminal up fresh: a remembered pane that has
     * since exited must fall through to the rule, and a copy taken when the
     * route was installed cannot.
     */
    fun paneOf(route: Route.Terminal): TerminalRef? {
        val terminals = terminalsIn(route.hostId, route.workspaceId)
        val key = Backstack.key(route.hostId, route.workspaceId)
        val id = Backstack.chooseFocus(terminals, _focus.value[key]) ?: return null
        return TerminalRef(route.hostId, route.workspaceId, id)
    }

    /** Where somebody was sent. Not written down — see [Focus]. */
    private fun point(ref: TerminalRef) = record(ref, chosen = false)

    /** Where somebody chose to be. Written down. */
    private fun rememberChoice(ref: TerminalRef) = record(ref, chosen = true)

    private fun record(ref: TerminalRef, chosen: Boolean) {
        val key = Backstack.key(ref.hostId, ref.workspaceId)
        val existing = _focus.value[key]
        if (existing?.terminalId == ref.terminalId && existing.chosen == chosen) return
        _focus.value = _focus.value + (key to Focus(ref.terminalId, chosen))
        // Only a choice changes what is on disk, so a fleet row tap costs
        // nothing here and cannot overwrite a real preference in the saved
        // copy. What that trades is narrow and deliberate: a pane you were sent
        // to and never confirmed does not come back after a process death — the
        // one you last chose in that workspace does.
        if (chosen) saved[FOCUS] = Backstack.encodeFocus(_focus.value)
    }

    /**
     * Cut the stack back to what still exists, and forget tabs for what does
     * not.
     *
     * Run on every fleet change rather than once at launch, because the two
     * cases are the same case: a workspace merged away while the app was dead
     * and a workspace merged away while somebody is looking at it both leave a
     * route naming nothing. See [Backstack.truncate] for why it truncates
     * rather than filters.
     */
    private fun settle() {
        val next = Backstack.truncate(_stack.value, ::resolves)
        if (next != _stack.value) install(next)

        val pruned = Backstack.prune(_focus.value) { key, terminalId ->
            val host = key.substringBefore('/')
            val workspace = key.substringAfter('/')
            // A runner that has not answered yet keeps its memory, for the same
            // reason its routes survive below: "not connected" is not "gone".
            if (!answered(host)) return@prune true
            terminalsIn(host, workspace).any { it.id == terminalId }
        }
        if (pruned != _focus.value) {
            _focus.value = pruned
            saved[FOCUS] = Backstack.encodeFocus(pruned)
        }
    }

    /**
     * Whether a route still names something.
     *
     * A runner that is merely reconnecting resolves. The question this asks is
     * "is it gone", and a slow SSH handshake is not gone — yanking somebody out
     * of the terminal they were reading because the Wi-Fi dropped would be a
     * worse bug than the one this rule fixes. So a route is only cut once the
     * runner has ANSWERED and does not have the thing, which is the same moment
     * iOS waits for.
     */
    private fun resolves(route: Route): Boolean = when (route) {
        is Route.Terminal -> {
            val configured = hosts.hosts.value.any { it.id == route.hostId }
            configured && (!answered(route.hostId) ||
                terminalsIn(route.hostId, route.workspaceId).isNotEmpty())
        }

        is Route.RunnerSettings -> hosts.hosts.value.any { it.id == route.hostId }

        // Nothing else names anything on a runner, so nothing else can stop
        // naming it.
        else -> true
    }

    private fun answered(hostId: String) =
        fleet.connection(hostId)?.phase?.value is Connection.Phase.Connected

    private fun terminalsIn(hostId: String, workspaceId: String): List<Terminal> =
        fleet.connection(hostId)?.fleet?.value?.workspaces
            ?.firstOrNull { it.id == workspaceId }?.terminals.orEmpty()

    private fun install(next: List<Route>, persist: Boolean = true) {
        // Never empty. There is no such thing as being nowhere, and an empty
        // stack is a blank screen with a back gesture that does nothing.
        val safe = next.ifEmpty { listOf(Backstack.ROOT) }
        if (safe == _stack.value) return
        _stack.value = safe
        _route.value = safe.last()
        if (persist) saved[STACK] = Backstack.encodeStack(safe)
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

    private companion object {
        // Namespaced, because a `SavedStateHandle` is one bundle shared with
        // anything else that ever writes to this activity's saved state.
        const val STACK = "farcooler.nav.stack"
        const val FOCUS = "farcooler.nav.focus"
        const val LANDED = "farcooler.nav.landed"
    }
}
