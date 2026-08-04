package com.farcooler.net

import com.farcooler.data.Host
import com.farcooler.data.HostStore
import com.farcooler.data.Settings
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
 * Which terminal a screen is looking at, on which machine.
 *
 * The host is carried rather than looked up. Short ids are the last eight hex
 * characters of a UUID minted per daemon; across three machines and a hundred
 * worktrees the birthday collision probability is around one in a hundred
 * thousand, and the cost of losing that coin flip is acting on the wrong
 * machine. Carrying the host removes the class of bug rather than betting
 * against it — the same conclusion the Mac reached.
 */
data class TerminalRef(val hostId: String, val workspaceId: String, val terminalId: String)

/** One workspace, and the machine it is on. */
data class FleetEntry(
    val host: Host,
    val connection: Connection,
    val workspace: Workspace,
)

/**
 * Every configured machine, connected at once.
 *
 * The Mac's `FleetStore`, on a phone. It holds one [Connection] per machine,
 * merges their workspaces into one list, and answers "which connection owns
 * this row". Every mutation goes through the connection this names.
 *
 * The iOS app has a machine picker instead, which makes a remote agent
 * something you have to go and look for — the product's whole claim is that an
 * agent blocked on a machine in another room is exactly as urgent as one on
 * this desk, and a picker answers that claim by asking you to switch machines
 * to find out. This is the fix, ported from the Mac design
 * (`docs/superpowers/specs/2026-08-03-every-machine-in-one-fleet-design.md`).
 *
 * **Failure isolation is structural.** One machine being unreachable is one
 * object in a bad state, not a flag threaded through shared code. That is the
 * reason to prefer a connection per machine over one that takes a host
 * parameter: there is somewhere natural for "this machine is down, here is why"
 * to live, and a row from a machine that stopped answering keeps its place
 * rather than vanishing.
 */
class FleetRepository(
    private val hosts: HostStore,
    private val settings: Settings,
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

    /** Every workspace on every connected machine, in machine order. */
    val entries: StateFlow<List<FleetEntry>> = _entries.asStateFlow()

    private val _connections = MutableStateFlow<List<Connection>>(emptyList())

    /** One per machine currently being talked to, in the order they are listed. */
    val active: StateFlow<List<Connection>> = _connections.asStateFlow()

    /** Reported once per fleet refresh, whichever machine produced it. */
    var onFleet: ((Host, com.farcooler.model.Fleet) -> Unit)? = null

    init {
        scope.launch {
            combine(hosts.hosts, hosts.selectedId, settings.allMachinesAtOnce) { all, selected, everything ->
                if (everything) all else all.filter { it.id == selected }
            }.collect { wanted -> reconcile(wanted) }
        }
    }

    /**
     * Bring up a connection per wanted machine and tear down the rest.
     *
     * Membership follows the host list, so adding a machine in settings brings
     * up a connection and removing one tears its connection down, with no
     * relaunch. A machine whose details were EDITED is torn down and rebuilt:
     * correcting a mistyped address is as much a change of machine as picking a
     * different one, and reusing the connection would leave the old session
     * running under new details.
     */
    private fun reconcile(wanted: List<Host>) {
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
            val connection = Connection(host, scope)
            connection.onFleet = { fleet -> onFleet?.invoke(host, fleet) }
            connections[host.id] = connection
            starts[host.id] = scope.launch { connection.start() }
            watchers[host.id] = listOf(
                scope.launch { connection.fleet.collect { publish() } },
                scope.launch { connection.phase.collect { publish() } },
            )
        }

        publish()
    }

    private fun publish() {
        val ordered = hosts.hosts.value.mapNotNull { connections[it.id] }
        _connections.value = ordered
        _entries.value = ordered.flatMap { connection ->
            connection.fleet.value.workspaces.map { workspace ->
                FleetEntry(connection.host, connection, workspace)
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

    /** Refresh every machine at once, for pull-to-refresh. */
    suspend fun refreshAll() {
        connections.values.forEach { it.refresh() }
    }

    /** Retry a machine that failed, without disturbing the others. */
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
    fun retry(hostId: String, withHost: Host) {
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
        connections.values.forEach { it.isForeground = foreground }
        if (foreground) scope.launch { refreshAll() }
    }

    /**
     * The terminal to open on, across every machine.
     *
     * The same rule one machine's fleet uses, applied to the union: an agent
     * waiting on you outranks everything else regardless of which machine it is
     * on, which is the entire point of connecting to all of them.
     */
    fun landing(): TerminalRef? {
        val all = _entries.value.flatMap { entry ->
            entry.workspace.terminals.map { entry to it }
        }
        all.firstOrNull { it.second.agent.wantsAttention }?.let { return it.ref() }
        all.firstOrNull {
            com.farcooler.model.StateKind.parse(it.second.state) ==
                com.farcooler.model.StateKind.RUNNING
        }?.let { return it.ref() }
        return all.firstOrNull()?.ref()
    }

    private fun Pair<FleetEntry, Terminal>.ref() =
        TerminalRef(first.host.id, first.workspace.id, second.id)
}
