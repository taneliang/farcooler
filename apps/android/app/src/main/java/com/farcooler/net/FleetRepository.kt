package com.farcooler.net

import com.farcooler.data.Runner
import com.farcooler.data.RunnerStore
import com.farcooler.data.Settings
import com.farcooler.model.InboxRow
import com.farcooler.model.NeedsYouInput
import com.farcooler.model.Terminal
import com.farcooler.model.Workspace
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

/**
 * Which terminal a screen is looking at, on which runner.
 *
 * The host is carried rather than looked up. Short ids are the last eight hex
 * characters of a UUID minted per daemon; across three runners and a hundred
 * workspaces the birthday collision probability is around one in a hundred
 * thousand, and the cost of losing that coin flip is acting on the wrong
 * runner. Carrying the runner removes the class of bug rather than betting
 * against it — the same conclusion the Mac reached.
 */
data class TerminalRef(val hostId: String, val workspaceId: String, val terminalId: String)

/** One workspace, the runner it is on, and what that runner said about its diff. */
data class FleetEntry(
    val host: Runner,
    val connection: Connection,
    val workspace: Workspace,
    /**
     * This worktree's `changes.inbox` row, or null while its runner has not
     * answered.
     *
     * Carried on the entry rather than looked up by the screens that want it,
     * for the same reason the runner is: this is the app's one merged,
     * already-observed list of workspaces across every runner, and a front door
     * that reached back into a per-connection map would have to subscribe to N
     * more flows and re-key every one of them by host. See [Connection.inbox]
     * for why that map is keyed by workspace alone.
     */
    val counts: InboxRow? = null,
) {
    /** This entry as the front door's derivation wants it. See `model/NeedsYou.kt`. */
    fun needsYouInput() = NeedsYouInput(host.id, host.displayLabel, workspace, counts)
}

/**
 * Every configured runner, connected at once.
 *
 * The Mac's `FleetStore`, on a phone. It holds one [Connection] per runner,
 * merges their workspaces into one list, and answers "which connection owns
 * this row". Every mutation goes through the connection this names.
 *
 * The iOS app has a runner picker instead, which makes a remote agent
 * something you have to go and look for — the product's whole claim is that an
 * agent blocked on a runner in another room is exactly as urgent as one on
 * this desk, and a picker answers that claim by asking you to switch runners
 * to find out. This is the fix, ported from the Mac design
 * (`docs/superpowers/specs/2026-08-03-every-machine-in-one-fleet-design.md`).
 *
 * **Failure isolation is structural.** One runner being unreachable is one
 * object in a bad state, not a flag threaded through shared code. That is the
 * reason to prefer a connection per runner over one that takes a runner
 * parameter: there is somewhere natural for "this runner is down, here is why"
 * to live, and a row from a runner that stopped answering keeps its place
 * rather than vanishing.
 */
class FleetRepository(
    private val hosts: RunnerStore,
    private val settings: Settings,
    /**
     * Handed to every [Connection] for its review stores.
     *
     * Threaded through rather than reached for, because this object is
     * deliberately given the stores it talks to and never the framework — see
     * the note on `AppModel.reachability`, which is the one thing that could not
     * be.
     */
    private val review: com.farcooler.data.ReviewStorage,
    private val scope: CoroutineScope,
) {
    private val connections = mutableMapOf<String, Connection>()
    private val starts = mutableMapOf<String, Job>()

    /**
     * What is watching each connection, so tearing one down takes its watchers
     * with it. A `StateFlow` never completes, so a collector on one lives as
     * long as the scope it was launched in — which here is the whole app.
     */
    private val watchers = mutableMapOf<String, List<Job>>()

    private val _entries = MutableStateFlow<List<FleetEntry>>(emptyList())

    /** Every workspace on every connected runner, in runner order. */
    val entries: StateFlow<List<FleetEntry>> = _entries.asStateFlow()

    private val _connections = MutableStateFlow<List<Connection>>(emptyList())

    /** One per runner currently being talked to, in the order they are listed. */
    val active: StateFlow<List<Connection>> = _connections.asStateFlow()

    /** Reported once per fleet refresh, whichever runner produced it. */
    var onFleet: ((Runner, com.farcooler.model.Fleet) -> Unit)? = null

    init {
        scope.launch {
            combine(hosts.hosts, hosts.selectedId, settings.allRunnersAtOnce) { all, selected, everything ->
                if (everything) all else all.filter { it.id == selected }
            }.collect { wanted -> reconcile(wanted) }
        }
    }

    /**
     * Bring up a connection per wanted runner and tear down the rest.
     *
     * Membership follows the runner list, so adding a runner in settings brings
     * up a connection and removing one tears its connection down, with no
     * relaunch. A runner whose details were EDITED is torn down and rebuilt:
     * correcting a mistyped address is as much a change of runner as picking a
     * different one, and reusing the connection would leave the old session
     * running under new details.
     */
    private fun reconcile(wanted: List<Runner>) {
        val byId = wanted.associateBy { it.id }

        for ((id, connection) in connections.toList()) {
            val host = byId[id]
            if (host != null && host == connection.host) continue
            connections.remove(id)
            starts.remove(id)?.cancel()
            watchers.remove(id)?.forEach { it.cancel() }
            scope.launch { connection.close() }
        }

        for (host in wanted) {
            if (connections.containsKey(host.id)) continue
            val connection = Connection(host, review, scope)
            connection.onFleet = { fleet -> onFleet?.invoke(host, fleet) }
            connections[host.id] = connection
            starts[host.id] = scope.launch { connection.start() }
            watchers[host.id] = listOf(
                scope.launch { connection.fleet.collect { publish() } },
                scope.launch { connection.phase.collect { publish() } },
                // The counts arrive on their own cadence — one read per
                // [Connection.INBOX_EVERY] fleet polls — so they need a watcher
                // of their own or a `+82 -13` would wait for the next fleet
                // change to reach the screen.
                scope.launch { connection.inbox.collect { publish() } },
            )
        }

        publish()
    }

    private fun publish() {
        val ordered = hosts.hosts.value.mapNotNull { connections[it.id] }
        _connections.value = ordered
        _entries.value = ordered.flatMap { connection ->
            val counts = connection.inbox.value
            connection.fleet.value.workspaces.map { workspace ->
                FleetEntry(connection.host, connection, workspace, counts[workspace.id])
            }
        }
    }

    fun connection(hostId: String): Connection? = connections[hostId]

    fun connection(ref: TerminalRef): Connection? = connections[ref.hostId]

    fun terminal(ref: TerminalRef): Terminal? =
        connections[ref.hostId]?.terminal(ref.terminalId)

    fun workspace(ref: TerminalRef): Workspace? =
        connections[ref.hostId]?.workspaceOf(ref.terminalId)

    fun entry(ref: TerminalRef): FleetEntry? =
        _entries.value.firstOrNull {
            it.host.id == ref.hostId && it.workspace.terminals.any { t -> t.id == ref.terminalId }
        }

    /**
     * End every session.
     *
     * Each connection owns a native handle and a runtime with threads of its
     * own; letting the process reclaim them is fine at exit and not fine when a
     * view model is cleared while the process lives on.
     */
    suspend fun close() {
        val open = connections.values.toList()
        connections.clear()
        starts.values.forEach { it.cancel() }
        starts.clear()
        watchers.values.flatten().forEach { it.cancel() }
        watchers.clear()
        open.forEach { it.close() }
        publish()
    }

    /**
     * Refresh every runner at once, for pull-to-refresh.
     *
     * Forced, so the diff counts come with it. Somebody pulling the list down
     * is asking for everything on it, and the one number on it that rides a
     * slower cadence is the one they would otherwise not get.
     */
    suspend fun refreshAll() {
        connections.values.forEach { it.refresh(force = true) }
    }

    /** Retry a runner that failed, without disturbing the others. */
    fun retry(hostId: String) {
        val connection = connections[hostId] ?: return
        starts[hostId]?.cancel()
        starts[hostId] = scope.launch { connection.start(connection.host) }
    }

    /**
     * Reconnect with a host key the user has just approved.
     *
     * The approved copy is passed in rather than re-read from the store: the
     * store deliberately does not write the fingerprint through to the selected
     * host, because that would rebuild the whole screen at the exact moment
     * approval succeeded.
     */
    fun retry(hostId: String, withHost: Runner) {
        val connection = connections[hostId] ?: return
        starts[hostId]?.cancel()
        starts[hostId] = scope.launch { connection.start(withHost) }
    }

    /**
     * Whether the app should be polling at all.
     *
     * Every connection stops when the app leaves the foreground: a poll is an
     * SSH round trip and a radio wake-up, and nothing is reading the answer.
     * The push path is what covers a phone in a pocket.
     */
    fun setForeground(foreground: Boolean) {
        // Each connection decides what coming back means for it — a healthy
        // one polls at once, a dropped one reconnects at once, one waiting on
        // a fingerprint does neither. That judgement needs the phase, which
        // lives there and not here.
        connections.values.forEach { it.setForeground(foreground) }
    }

    /**
     * Retry every runner at once, whatever each one's backoff had planned.
     *
     * What the network coming back means. Only the transition into reachable
     * reaches this — see [Reachability] — because a path that was already
     * satisfied and stayed that way is not news, and a phone hands out plenty
     * of those.
     */
    fun reconnectAll() {
        connections.values.forEach { it.reconnectNow() }
    }

    // There is no fleet-wide landing rule here any more, and its absence is
    // the point.
    //
    // `landing()` used to answer "which terminal does the app open on", merging
    // every runner's panes and applying `model/Model.kt`'s ordering to the
    // union — the cross-runner form of a rule iOS applies to one connection.
    // The front door replaced the question rather than the answer: the app no
    // longer opens onto a terminal at all, so nothing asks which one. What was
    // cross-runner about it survives, better, in `model/NeedsYou.kt`, which
    // merges the same union and orders it by the host's own `rank` instead of
    // by first-match.
    //
    // The narrower rule is untouched: `List<Terminal>.landingTerminal` still
    // decides which PANE a workspace opens on, through `Backstack.rule`.
}
