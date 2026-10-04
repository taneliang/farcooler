package com.farcooler.net

import com.farcooler.model.PlanPage
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanRecord
import com.farcooler.model.WorkspaceSummary
import com.farcooler.core.CoreException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * The plan layer, as one connection reads it (ov-274): a board's plan, and a
 * theme's or lane's record. EXPERIMENTAL and opt-in: nothing is asked of a
 * runner without `board_plan`, and nothing until someone picks Plan on a
 * board. The rules are `model/Plan.kt`'s; this reads over the connection and
 * keeps the answers, as the iPhone's `ConnectionPlan.swift` does.
 *
 * Every read lands in a state ([PlanReadState.read]): the plan, an update to
 * ask for, or a read that failed or was never answered. Never a spinner for
 * good. Removing the layer means deleting this file, the Plan views, the plan
 * route and the call sites that name `plans`.
 */
class PlanReads(
    private val call: suspend (method: String, args: JsonObject) -> JsonObject,
    /** Whether the runner advertises `board_plan`; null before its build is read. */
    private val runnerCan: () -> Boolean?,
    private val timeoutMs: Long = PlanReadState.TIMEOUT_MS,
) {
    private val _states = MutableStateFlow<Map<String, PlanReadState>>(emptyMap())

    /** Each board's plan read, by workspace id. */
    val states: StateFlow<Map<String, PlanReadState>> = _states.asStateFlow()

    private val _records = MutableStateFlow<Map<PlanPage, PlanRecord>>(emptyMap())

    /** Each theme's and lane's record, by page, once read. */
    val records: StateFlow<Map<PlanPage, PlanRecord>> = _records.asStateFlow()

    /** Whether this runner keeps a plan: only then is there a control. */
    val keeps: Boolean get() = runnerCan() == true


    private val reading = mutableSetOf<String>()
    private val movedAgain = mutableSetOf<String>()

    /**
     * The theme's or lane's page on screen, if one is: the only record a plan
     * notice reads again, rather than every page opened since.
     */
    @Volatile var openPage: PlanPage? = null

    /** Read [workspace]'s board's plan. One at a time per board, and once more after it if news came meanwhile. */
    suspend fun read(workspace: WorkspaceSummary) {
        val key = workspace.id
        if (!reading.add(key)) {
            movedAgain += key
            return
        }
        try {
            if ((_states.value[key] as? PlanReadState.Loaded) == null) set(key, PlanReadState.Loading)
            do {
                movedAgain -= key
                val board = workspace.boardWorkspace
                if (board == null) {
                    // An implicit board has no plan to ask for: not an old runner, so not "needs an update".
                    set(key, PlanReadState.Unavailable)
                    return
                }
                val state = PlanReadState.read(
                    runnerCan = runnerCan(),
                    timeoutMs = timeoutMs,
                    isUnsupported = { (it as? CoreException)?.word == "capability-unsupported" },
                ) { call("plan.get", buildJsonObject { put("workspace", board) }).toString() }
                // A read that fails over a plan already in hand keeps the plan:
                // the screen draws it, and the next notice reads again.
                if (state == PlanReadState.Unavailable && _states.value[key] is PlanReadState.Loaded) break
                set(key, state)
            } while (movedAgain.contains(key))
        } finally {
            reading -= key
        }
    }

    /** A theme's or lane's record, for its page's timeline. One that doesn't come back leaves the page without; it doesn't wait on it. */
    suspend fun readRecord(page: PlanPage) {
        if (runnerCan() != true) return
        val subject = buildJsonObject {
            when (page) {
                is PlanPage.Theme -> put("theme", page.id)
                is PlanPage.Lane -> put("lane", page.id)
            }
        }
        val record = try {
            PlanRecord.decode(call("plan.events", subject))
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (e: Exception) {
            return
        }
        if (_records.value[page] != record) _records.value = _records.value + (page to record)
    }

    /** News that [board]'s plan moved: if its plan was asked for, read it again, and every open record. */
    suspend fun heard(board: String, boards: List<WorkspaceSummary>) {
        for (workspace in boards) {
            if (workspace.id.equals(board, ignoreCase = true) && _states.value.containsKey(workspace.id)) {
                read(workspace)
                openPage?.let { readRecord(it) }
            }
        }
    }

    /** A runner notice, as [Connection] hands it over: a plan notice reads that board's plan again, while the app is in front. */
    suspend fun noticed(notice: JsonObject, boards: List<WorkspaceSummary>, foreground: Boolean) {
        com.farcooler.model.PlanNews.board(notice)?.let { if (foreground) heard(it, boards) }
    }

    /** Boards a task or resync notice moved: their counts and "needs you" are the board's statuses, derived on read. */
    suspend fun reread(boards: List<WorkspaceSummary>) {
        for (workspace in boards) if (_states.value.containsKey(workspace.id)) read(workspace)
    }

    private fun set(key: String, state: PlanReadState) {
        if (_states.value[key] != state) _states.value = _states.value + (key to state)
    }
}
