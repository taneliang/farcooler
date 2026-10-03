package com.farcooler.model

/**
 * Where a [Destination] actually opens, given what this phone holds now
 * (ov-182, ov-183). AgentKit's `DestinationResolver`, mirrored case for case,
 * and held to the same `test/fixtures/destinations.json`.
 *
 * 1. **Find the runner**: by host, then by id among ready runners (two
 *    seats with one id are one runner), waiting while any runner hasn't said
 *    its id. A paired runner nothing is dialing is answered
 *    [Resolution.Connect]. One naming no runner is looked for on every ready
 *    runner, and refused when two have it.
 * 2. **Walk the place**: not known yet waits; gone falls back a level,
 *    quietly (terminal → worktree → workspace → the runner's first
 *    workspace). Finer parts are dropped one at a time when stale.
 * 3. **At the deadline**, a restore opens the deepest level that's here, then
 *    the last workspace, the first, Needs You; a notification stays, with a
 *    [Note], and never opens past its deadline.
 * 4. **Moving first wins** over a restore.
 *
 * A notification never falls back past its workspace.
 */
object DestinationResolver {
    enum class Arrival(val wire: String) { RESTORE("restore"), NOTIFICATION("notification") }

    /** How long each arrival waits, in milliseconds. */
    object Deadline {
        /** [com.farcooler.ui.LaunchRule.WINDOW_MS]. */
        const val RESTORE_MS = 10_000L
        /** A phone's cold launch can take most of a minute to reach a runner. */
        const val NOTIFICATION_MS = 60_000L
    }

    /** Why a notification left the app where it was. */
    enum class Note(val wire: String) {
        RUNNER_UNAVAILABLE("runner-unavailable"),
        NOT_FOUND("not-found"),
        GONE("gone"),
        AMBIGUOUS("ambiguous"),
    }

    sealed interface Resolution {
        data object Wait : Resolution
        /** Open [destination]; [fellBack] when it isn't the level asked for. */
        data class Open(val destination: Destination, val fellBack: Boolean) : Resolution
        /** Do nothing; a notification says why. */
        data class Stay(val note: Note?) : Resolution
        /** Its runner is paired but nothing is dialing it: dial [host], then ask again. */
        data class Connect(val host: String) : Resolution
    }

    /** What this phone holds now. Null collections are not read yet. */
    data class World(val seats: List<Seat>, val lastWorkspace: Last? = null) {
        data class Last(val host: String, val workspace: String)

        data class Seat(
            val host: String,
            val runnerId: String? = null,
            val ready: Boolean,
            /** Paired but not connected, and nothing is dialing it. */
            val idle: Boolean = false,
            /** In the switcher's order. */
            val workspaces: List<Workspace>? = null,
            val worktrees: List<Worktree>? = null,
            /** Boards read so far, by workspace id. */
            val boards: Map<String, List<Task>> = emptyMap(),
            /** Task ids or keys a direct lookup said this runner doesn't have. */
            val absentTasks: List<String> = emptyList(),
        ) {
            /** Coming up on its own: worth waiting for. */
            val dialing: Boolean get() = !ready && !idle
        }

        data class Workspace(val id: String, val orchestrator: Boolean = true)
        data class Worktree(val id: String, val workspace: String? = null, val terminals: List<Terminal> = emptyList())
        data class Terminal(val id: String, val orchestrator: Boolean = false)
        data class Task(val id: String, val key: String? = null, val repository: String? = null, val tabs: List<String>? = null)
    }

    private enum class Presence { HERE, GONE, UNKNOWN }

    fun resolve(
        destination: Destination,
        arrival: Arrival,
        world: World,
        elapsedMs: Long,
        deadlineMs: Long,
        interrupted: Boolean = false,
    ): Resolution {
        if (arrival == Arrival.RESTORE && interrupted) return Resolution.Stay(null)
        val late = elapsedMs >= deadlineMs
        val resolution = decide(destination, arrival, world, late)
        // A notification past its deadline opens nothing, even when what it's
        // about has just turned up (ov-106).
        if (arrival == Arrival.NOTIFICATION && late && resolution is Resolution.Open) return Resolution.Stay(Note.NOT_FOUND)
        return resolution
    }

    private fun decide(destination: Destination, arrival: Arrival, world: World, late: Boolean): Resolution {
        val restore = arrival == Arrival.RESTORE
        if (destination.place == Destination.Place.NeedsYou) return Resolution.Open(Destination.NEEDS_YOU, false)
        val unavailable = { if (restore) general(world) else Resolution.Stay(Note.RUNNER_UNAVAILABLE) }

        val runner = destination.runner
        val byHost = runner.host?.let { host -> world.seats.firstOrNull { it.host == host } }
        if (byHost != null) {
            if (byHost.ready) return walk(destination, byHost, arrival, world, late)
            if (late) return unavailable()
            return if (byHost.idle) Resolution.Connect(byHost.host) else Resolution.Wait
        }
        val id = runner.id?.lowercase()
        if (id == null) {
            if (runner.host != null) return unavailable()
            return search(destination, arrival, world, late)
        }
        // Two seats with one id are one runner reached two ways: the first
        // that has the place, else the first.
        val matching = world.seats.filter { it.ready && it.runnerId?.lowercase() == id }
        val seat = matching.firstOrNull { presence(destination.place, it) == Presence.HERE } ?: matching.firstOrNull()
        if (seat != null) return walk(destination, seat, arrival, world, late)
        val idle = world.seats.firstOrNull { it.idle && it.runnerId?.lowercase() == id }
        return when {
            late -> unavailable()
            idle != null -> Resolution.Connect(idle.host)
            world.seats.any { it.dialing } -> Resolution.Wait
            else -> unavailable()
        }
    }

    private fun search(destination: Destination, arrival: Arrival, world: World, late: Boolean): Resolution {
        val found = mutableListOf<Resolution>()
        var unknown = world.seats.any { it.dialing }
        for (seat in world.seats.filter { it.ready }) {
            when (presence(destination.place, seat)) {
                Presence.HERE -> found += walk(destination, seat, arrival, world, late)
                Presence.UNKNOWN -> unknown = true
                Presence.GONE -> Unit
            }
        }
        val restore = arrival == Arrival.RESTORE
        if (found.size == 1) return found[0]
        if (found.size > 1) return if (restore) general(world) else Resolution.Stay(Note.AMBIGUOUS)
        if (unknown && !late) return Resolution.Wait
        if (restore) return general(world)
        return Resolution.Stay(if (unknown) Note.NOT_FOUND else Note.GONE)
    }

    private fun presence(place: Destination.Place, seat: World.Seat): Presence = when (place) {
        Destination.Place.NeedsYou -> Presence.HERE
        is Destination.Place.Workspace -> seat.workspaces?.let { list ->
            if (list.any { it.id == place.id }) Presence.HERE else Presence.GONE
        } ?: Presence.UNKNOWN
        is Destination.Place.Orchestrator -> seat.workspaces?.let { list ->
            if (list.any { it.id == place.workspace && it.orchestrator }) Presence.HERE else Presence.GONE
        } ?: Presence.UNKNOWN
        is Destination.Place.History -> presence(Destination.Place.Workspace(place.workspace), seat)
        is Destination.Place.Task ->
            if (findTask(seat, place.task, place.workspace) != null) Presence.HERE
            else taskAbsence(place.task, place.workspace, seat)
        is Destination.Place.Worktree -> seat.worktrees?.let { list ->
            if (list.any { it.id == place.id }) Presence.HERE else Presence.GONE
        } ?: Presence.UNKNOWN
        is Destination.Place.Terminal -> seat.worktrees?.let { list ->
            if (list.any { tree -> tree.terminals.any { it.id == place.id } }) Presence.HERE else Presence.GONE
        } ?: Presence.UNKNOWN
    }

    private fun taskAbsence(ref: Destination.TaskRef, workspace: String?, seat: World.Seat): Presence {
        if (ref.id != null && ref.id in seat.absentTasks) return Presence.GONE
        if (ref.key != null && ref.key in seat.absentTasks) return Presence.GONE
        if (workspace != null) {
            // A task on a workspace that's gone is gone with it.
            if (presence(Destination.Place.Workspace(workspace), seat) == Presence.GONE) return Presence.GONE
            return if (seat.boards[workspace] == null) Presence.UNKNOWN else Presence.GONE
        }
        val workspaces = seat.workspaces ?: return Presence.UNKNOWN
        return if (workspaces.all { seat.boards[it.id] != null }) Presence.GONE else Presence.UNKNOWN
    }

    private fun walk(destination: Destination, seat: World.Seat, arrival: Arrival, world: World, late: Boolean): Resolution {
        val runner = Destination.Runner(host = seat.host, id = seat.runnerId)
        val ladder = listOf(destination.place) + destination.place.ancestors
        for ((depth, place) in ladder.withIndex()) {
            when (presence(place, seat)) {
                Presence.GONE -> continue
                Presence.UNKNOWN -> {
                    if (!late) return Resolution.Wait
                    if (arrival == Arrival.NOTIFICATION) return Resolution.Stay(Note.NOT_FOUND)
                    continue
                }
                Presence.HERE -> {
                    if (depth == 0) return Resolution.Open(refine(destination, seat, runner), false)
                    val segment = destination.segment?.takeIf { place is Destination.Place.Workspace && keeps(it, place, seat) }
                    return Resolution.Open(Destination(runner = runner, place = place, segment = segment), true)
                }
            }
        }
        if (arrival == Arrival.NOTIFICATION) return Resolution.Stay(Note.GONE)
        seat.workspaces?.firstOrNull()?.let {
            return Resolution.Open(Destination(runner = runner, place = Destination.Place.Workspace(it.id)), true)
        }
        return general(world)
    }

    private fun refine(destination: Destination, seat: World.Seat, runner: Destination.Runner): Destination {
        var out = destination.copy(runner = runner)
        when (val place = destination.place) {
            is Destination.Place.Task -> findTask(seat, place.task, place.workspace)?.let { (row, workspace) ->
                out = out.copy(
                    place = Destination.Place.Task(
                        workspace,
                        Destination.TaskRef(id = row.id, key = row.key ?: place.task.key, repository = row.repository ?: place.task.repository),
                    ),
                    tab = out.tab?.takeIf { tab -> row.tabs == null || tab.wire in row.tabs },
                )
            }
            is Destination.Place.Terminal -> findTerminal(seat, place.id)?.let { (worktree, terminal) ->
                val workspace = worktree.workspace
                out = out.copy(
                    place = if (terminal.orchestrator && workspace != null) Destination.Place.Orchestrator(workspace)
                    else Destination.Place.Worktree(worktree.id, workspace),
                    pane = place.id,
                )
            }
            else -> Unit
        }
        if (out.place !is Destination.Place.Task) out = out.copy(tab = null, agent = null, question = false)
        out = if (out.place is Destination.Place.Workspace) {
            out.copy(segment = out.segment?.takeIf { keeps(it, out.place, seat) })
        } else {
            out.copy(segment = null)
        }
        // Not read yet is kept: the platform checks it again as it lands.
        if (seat.worktrees != null) {
            if (out.pane != null && findTerminal(seat, out.pane!!) == null) out = out.copy(pane = null)
            if (out.agent != null && findTerminal(seat, out.agent!!) == null) out = out.copy(agent = null)
        }
        return out
    }

    private fun keeps(segment: Destination.Segment, place: Destination.Place, seat: World.Seat): Boolean {
        if (segment != Destination.Segment.ORCHESTRATOR || place !is Destination.Place.Workspace) return true
        return seat.workspaces?.any { it.id == place.id && it.orchestrator } ?: false
    }

    private fun general(world: World): Resolution {
        world.lastWorkspace?.let { last ->
            val seat = world.seats.firstOrNull { it.ready && it.host == last.host }
            if (seat != null && presence(Destination.Place.Workspace(last.workspace), seat) == Presence.HERE) {
                return Resolution.Open(
                    Destination(Destination.Runner(seat.host, seat.runnerId), Destination.Place.Workspace(last.workspace)), true,
                )
            }
        }
        for (seat in world.seats.filter { it.ready }) {
            val first = seat.workspaces?.firstOrNull() ?: continue
            return Resolution.Open(Destination(Destination.Runner(seat.host, seat.runnerId), Destination.Place.Workspace(first.id)), true)
        }
        return Resolution.Open(Destination.NEEDS_YOU, true)
    }

    /** The row matching [ref] and the workspace it's on. */
    private fun findTask(seat: World.Seat, ref: Destination.TaskRef, workspace: String?): Pair<World.Task, String>? {
        val names = workspace?.let(::listOf) ?: seat.workspaces?.map { it.id } ?: seat.boards.keys.sorted()
        for (name in names) {
            seat.boards[name].orEmpty().firstOrNull { matches(it, ref) }?.let { return it to name }
        }
        return null
    }

    private fun matches(row: World.Task, ref: Destination.TaskRef): Boolean {
        if (ref.id != null) return row.id == ref.id
        if (ref.key == null || row.key != ref.key) return false
        val repository = ref.repository ?: return true
        val own = row.repository ?: return true
        return own.lowercase() == repository.lowercase()
    }

    private fun findTerminal(seat: World.Seat, id: String): Pair<World.Worktree, World.Terminal>? {
        for (worktree in seat.worktrees.orEmpty()) {
            worktree.terminals.firstOrNull { it.id == id }?.let { return worktree to it }
        }
        return null
    }
}
