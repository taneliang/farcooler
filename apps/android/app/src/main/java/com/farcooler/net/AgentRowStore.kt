package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.model.AgentRow
import com.farcooler.model.AgentRowChanges
import com.farcooler.model.AgentRowFollowed
import com.farcooler.model.AgentRowLedger
import com.farcooler.model.AgentRowPage
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject

/**
 * Where a terminal's agent rows come from: `agent.rows` and `agent.rows_follow`
 * on some client, answering the client core's JSON
 * (`crates/client/src/ffi/rows_args.rs`).
 */
interface AgentRowSource {
    /** The newest rows, or up to [limit] before [before]. */
    suspend fun page(before: Long?, limit: Int): JsonObject

    /** What changed after [afterRev], the runner holding the call up to [waitMs] for something to. */
    suspend fun follow(epoch: Long, afterRev: Long, waitMs: Int): JsonObject
}

/**
 * A source's failure that retrying won't fix: the runner doesn't serve rows
 * (no `agent_rows`, the projector flag off), or the pane is gone.
 */
class AgentRowsUnavailable : Exception("The runner doesn’t serve this pane’s rows.")

/** A terminal's rows over this phone's client core: `agent.rows` and `agent.rows_follow`. */
class CoreRowSource(private val core: ClientCall, private val terminal: String) : AgentRowSource {
    override suspend fun page(before: Long?, limit: Int): JsonObject = mapped {
        core.call(
            "agent.rows",
            kotlinx.serialization.json.buildJsonObject {
                put("terminal", kotlinx.serialization.json.JsonPrimitive(terminal))
                put("limit", kotlinx.serialization.json.JsonPrimitive(limit))
                if (before != null) put("before", kotlinx.serialization.json.JsonPrimitive(before))
            },
        )
    }

    override suspend fun follow(epoch: Long, afterRev: Long, waitMs: Int): JsonObject = mapped {
        core.call(
            "agent.rows_follow",
            kotlinx.serialization.json.buildJsonObject {
                put("terminal", kotlinx.serialization.json.JsonPrimitive(terminal))
                put("epoch", kotlinx.serialization.json.JsonPrimitive(epoch))
                put("afterRev", kotlinx.serialization.json.JsonPrimitive(afterRev))
                put("waitMs", kotlinx.serialization.json.JsonPrimitive(waitMs))
            },
        )
    }

    /**
     * A runner that doesn't serve rows (its projector turned off since the
     * hello), or a pane that's gone, is not worth retrying.
     */
    private suspend fun mapped(call: suspend () -> JsonObject): JsonObject = try {
        call()
    } catch (e: CoreException) {
        if (e.word == "capability-unsupported" || e.word == "not-found") throw AgentRowsUnavailable()
        throw e
    }
}

/** The one call a source and a sink make: the client core's `call`, so a test can stand in for it. */
fun interface ClientCall {
    suspend fun call(method: String, args: JsonObject): JsonObject
}

/**
 * A terminal's rows for a view, fed from off the main thread: AgentKit's
 * `AgentRowStore` (ov-371).
 *
 * The loop pages, follows, and pages again whenever the runner says it can't
 * diff (a reset, a new epoch) and after any failure, since a call that failed
 * may have lost changes nobody can name. The ledger outlives a stop, so a pane
 * switched away from and back follows from where it was and asks the runner
 * only what changed.
 *
 * [retryDelayMs] is the first wait after a failed call, doubled on each failure
 * in a row; tests set it near zero.
 */
class AgentRowStore(
    private val scope: CoroutineScope,
    private val retryDelayMs: Long = 500,
    private val followWaitMs: Int = FOLLOW_WAIT_MS,
) {
    sealed interface Phase {
        /** Nothing to draw yet. */
        data object Loading : Phase

        /** Drawn from what was held; the runner hasn't answered since. */
        data object Cached : Phase

        /** Following the runner. */
        data object Live : Phase

        /** The runner doesn't serve rows for this pane. */
        data object Unavailable : Phase

        /** The last call failed; it's being retried. */
        data class Trouble(val words: String) : Phase
    }

    /** What the view draws. */
    data class Shown(
        /** Every held row, oldest first. A row that didn't change is the same object. */
        val rows: List<AgentRow> = emptyList(),
        val phase: Phase = Phase.Loading,
        /** Whether older rows exist than the oldest held. */
        val moreBefore: Boolean = false,
        val loadingOlder: Boolean = false,
    ) {
        /**
         * The rows held may be out of date: the last call failed, or the runner
         * stopped serving them. A view says so over them rather than letting
         * them look live.
         */
        val isStale: Boolean get() = phase is Phase.Trouble || phase is Phase.Unavailable
    }

    private val _shown = MutableStateFlow(Shown())
    val shown: StateFlow<Shown> = _shown.asStateFlow()

    val ledger = AgentRowLedger()
    private var feed: Job? = null

    /** The loop is running. One that ended because the runner stopped serving rows is not. */
    val isFollowing: Boolean get() = feed?.isActive == true

    /** Start following [source]; a running follow is replaced. */
    fun start(source: AgentRowSource) {
        feed?.cancel()
        // What was held is drawn at once, as held rather than as live.
        _shown.update { shown ->
            shown.copy(phase = if (shown.rows.isEmpty()) Phase.Loading else Phase.Cached, loadingOlder = false)
        }
        feed = scope.launch { run(source) }
    }

    fun stop() {
        feed?.cancel()
        feed = null
    }

    /** Ask for the page above the oldest held row, once at a time. */
    fun loadOlder(source: AgentRowSource) {
        val now = _shown.value
        if (!now.moreBefore || now.loadingOlder) return
        _shown.update { it.copy(loadingOlder = true) }
        scope.launch {
            try {
                val oldest = ledger.oldestOrd
                if (oldest != null) {
                    // Published even when no row came: the page also says whether more exist.
                    ledger.older(AgentRowPage.decode(source.page(oldest, PAGE_SIZE)))
                    publish()
                }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // A page that didn't come is asked for again when the top shows.
            } finally {
                _shown.update { it.copy(loadingOlder = false) }
            }
        }
    }

    private fun publish(phase: Phase? = null) {
        _shown.update { shown ->
            shown.copy(rows = ledger.held(), phase = phase ?: shown.phase, moreBefore = ledger.moreBefore)
        }
    }

    private fun set(phase: Phase) {
        _shown.update { if (it.phase == phase) it else it.copy(phase = phase) }
    }

    private suspend fun run(source: AgentRowSource) {
        var needsPage = ledger.cursor == null
        var waitMs = 0
        var backoff = retryDelayMs
        while (currentCoroutineContext().isActive) {
            try {
                if (needsPage) {
                    ledger.replace(AgentRowPage.decode(source.page(null, PAGE_SIZE)))
                    needsPage = false
                    publish()
                } else {
                    val (epoch, rev) = ledger.cursor ?: (0L to 0L)
                    when (val followed = ledger.apply(AgentRowChanges.decode(source.follow(epoch, rev, waitMs)))) {
                        AgentRowFollowed.Reset -> {
                            needsPage = true
                            continue
                        }
                        is AgentRowFollowed.Changed -> if (!followed.delta.isEmpty) publish()
                    }
                }
                waitMs = followWaitMs
                backoff = retryDelayMs
                set(Phase.Live)
            } catch (e: CancellationException) {
                throw e
            } catch (_: AgentRowsUnavailable) {
                set(Phase.Unavailable)
                return
            } catch (e: Exception) {
                set(Phase.Trouble(e.message.orEmpty()))
                needsPage = true
                delay(backoff)
                backoff = minOf(backoff * 2, 10_000)
            }
        }
    }

    companion object {
        /** Rows a page asks for. */
        const val PAGE_SIZE = 100

        /** How long a follow is held when nothing changes. */
        const val FOLLOW_WAIT_MS = 20_000
    }
}
