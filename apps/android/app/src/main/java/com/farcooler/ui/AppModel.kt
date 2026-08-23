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
import com.farcooler.data.PreferenceReviewStorage
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

    /**
     * Where a review's bookmark and its unsent notes are written down.
     *
     * Here rather than inside [FleetRepository] for the same reason
     * [reachability] is here: it needs a `Context`, and the repository is given
     * the stores it talks to rather than the framework.
     */
    private val reviewStorage = PreferenceReviewStorage(application)

    val fleet = FleetRepository(hosts, settings, reviewStorage, viewModelScope)

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
     * it away in the one case restoration exists for. A restored `[NeedsYou]`
     * is indistinguishable from a cold start without this.
     *
     * Now that the front door is the root there is only one decision left for
     * it to hold — onboarding or not — so the window it protects is much
     * narrower than it was. Kept, because that decision is still one somebody
     * can move: [addHost] clears it deliberately, so adding the first runner
     * from onboarding leaves onboarding.
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
     * The one decision left before the first screen: onboarding, or the front
     * door.
     *
     * This used to choose a TERMINAL. It merged every runner's panes, applied
     * the landing rule to the union and opened the winner, with the workspace
     * list as the fallback for a fleet with nothing running — and it had to
     * wait for a runner to answer before it could, because a slow SSH handshake
     * would otherwise land on an empty list and stay there.
     *
     * All of that is gone with the front door, and the waiting went with it.
     * [Route.NeedsYou] needs nothing from any runner to be worth showing: with
     * no answer yet it says every runner is connecting, which is both true and
     * the thing somebody opening the app during a handshake wants to know.
     * There is no landing decision to get wrong and no window in which to get
     * it wrong.
     *
     * What is left is the case the old comment was written for and which is
     * still exactly right: **a list of runners is onboarding, and onboarding is
     * not a home screen** — so it appears exactly when it is the thing to do,
     * which is when there are no runners.
     */
    fun landIfNeeded() {
        // Before anything that reads the stack: a route naming a workspace that
        // is gone has to be off the stack before anything decides whether the
        // app has somewhere to be.
        settle()

        // A notification tap outranks the front door: somebody who tapped
        // "claude needs you" asked for that pane by name.
        resolvePendingTerminal()
        if (_landed.value) return
        land(if (hosts.hosts.value.isEmpty()) Route.Onboarding else Backstack.ROOT)
    }

    private fun land(route: Route) {
        _landed.value = true
        saved[LANDED] = true
        install(listOf(route))
    }

    /**
     * Go to a pane somebody was SENT to — a front-door row, a fleet row, the
     * drawer, a push notification.
     *
     * Pushes onto whatever sent you, and swaps one terminal for another rather
     * than stacking them. See [goTo].
     */
    fun open(ref: TerminalRef) {
        point(ref)
        goTo(Route.Terminal(ref.hostId, ref.workspaceId))
    }

    /**
     * Go to a WORKSPACE, without naming a pane in it.
     *
     * The front door's "Review changes" row, which is about the worktree rather
     * than about any agent in it. Deliberately does not [point] at anything:
     * `TerminalRef` would need a terminal id and there is no honest one to
     * give, and writing an empty one into the focus map would put a value in
     * there that resolves to nothing on every later read.
     *
     * So the workspace's own rule chooses the pane — the remembered tab if it
     * still lives, whatever needs you otherwise. See [paneOf].
     */
    fun openWorkspace(hostId: String, workspaceId: String) {
        goTo(Route.Terminal(hostId, workspaceId))
    }

    /**
     * Go to a tab somebody CHOSE — the one writer of the remembered focus.
     *
     * Called from the tab strip and nowhere else, and **the stack is never
     * touched**. That last clause used to be conditional: the strip spanned the
     * whole fleet, so a chip could belong to another workspace on another
     * runner and tapping it was navigation. The strip is scoped to one
     * workspace now, so every chip on it names the workspace already on screen
     * and [goTo] finds its own target at the top of the stack — which is the
     * property the whole shape exists for. Tapping a chip moves no navigation
     * state, so nothing keyed on the route has a reason to reset, and the panes
     * mounted under it are not disturbed. See [Route.Terminal].
     *
     * Takes the runner and the workspace beside the tab rather than a
     * `TerminalRef`, because the Changes tab is a tab with no terminal in it and
     * a `TerminalRef` cannot say so. Everything that names something on a runner
     * still carries the runner.
     *
     * Choosing the chip you are already on still records. Confirming the rule's
     * guess is a choice, and the next visit should not have to guess again.
     */
    fun choose(hostId: String, workspaceId: String, pane: Pane) {
        record(Backstack.key(hostId, workspaceId), pane, chosen = true)
        goTo(Route.Terminal(hostId, workspaceId))
    }

    /**
     * Put a workspace on screen, over whatever sent you there.
     *
     * **A workspace PUSHES now, where it used to replace the whole stack.** The
     * old rule — and the comment that argued for it — was right about the app
     * it was written in: the terminal was the home screen, a home screen with a
     * back button that goes somewhere is not one, and the workspace list was an
     * edge swipe away in the drawer. Every clause of that is false now that
     * [Route.NeedsYou] is the root. A terminal opened from the front door has
     * somewhere to go back to and it is the screen that sent you, which is the
     * whole reason the front door is worth having.
     *
     * **A terminal replaces a terminal.** Tapping another workspace in the
     * drawer while already in one must not stack them: the second Back would
     * then land on a workspace nobody asked to see again. So the trailing run
     * of terminals is dropped and one is put back — which leaves the front door
     * one Back away wherever you got here from, and leaves the workspace list
     * in between when it was the thing that sent you. That is iOS's depth-2
     * shape, and the Back bug `43a320f` names is precisely the version of this
     * that replaced instead of appending.
     *
     * Arriving at the workspace already underneath only closes what is over it,
     * so a push for a pane in the workspace you are already in costs no
     * navigation at all — which is what keeps a tab tap free. See [choose].
     */
    private fun goTo(target: Route.Terminal) {
        val base = _stack.value.dropLastWhile { it.isOverlay }
        if (base.lastOrNull() == target) {
            install(base)
            return
        }
        install(base.dropLastWhile { it is Route.Terminal } + target)
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

    /**
     * Back to the front door, whatever is stacked up.
     *
     * Was `showFleet`, and the rename is the whole of the change: [Backstack.ROOT]
     * has always been what it installed, and ROOT is no longer the workspace
     * list. A method named for its old destination is how the next reader ends
     * up somewhere else.
     */
    fun goHome() {
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
     * `TerminalPane` looks its terminal up fresh: a remembered pane that has
     * since exited must fall through to the rule, and a copy taken when the
     * route was installed cannot.
     */
    fun paneOf(route: Route.Terminal): Pane? {
        val terminals = terminalsIn(route.hostId, route.workspaceId)
        val key = Backstack.key(route.hostId, route.workspaceId)
        return Backstack.chooseFocus(terminals, _focus.value[key])
    }

    /** Where somebody was sent. Not written down — see [Focus]. */
    private fun point(ref: TerminalRef) =
        record(
            Backstack.key(ref.hostId, ref.workspaceId),
            Pane.Terminal(ref.terminalId),
            chosen = false,
        )

    private fun record(key: String, pane: Pane, chosen: Boolean) {
        val existing = _focus.value[key]
        if (existing?.pane == pane && existing.chosen == chosen) return
        _focus.value = _focus.value + (key to Focus(pane, chosen))
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

    private val _foreground = MutableStateFlow(true)

    /**
     * Whether the app is in front of somebody.
     *
     * Observable, which it did not have to be until panes started staying
     * mounted. [FleetRepository] and [Notifier] are TOLD this and act on it;
     * the workspace screen has to be able to WATCH it, because every mounted
     * pane's stream and agent poll follow it. Without that, a phone with three
     * tabs open would hold three second SSH channels while it sat in a pocket —
     * where one re-pointed session held one, and the push path is what covers a
     * phone nobody is looking at anyway.
     *
     * Not saved. "Is the app in front of me" is answered by the activity's own
     * lifecycle the moment there is one, and a restored `true` from a process
     * that has since died would be a claim about nothing.
     */
    val foreground: StateFlow<Boolean> = _foreground.asStateFlow()

    fun setForeground(foreground: Boolean) {
        _foreground.value = foreground
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
