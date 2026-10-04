package com.farcooler.model

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * One board's read state on this phone (ov-113): what Unread and Done read, kept
 * on the runner when the runner keeps it, and on this phone when it can't.
 * AgentKit's `BoardReadsKeeper`, rule for rule:
 *
 * - **Every value only rises.** A state heard late, or lower, changes nothing.
 * - **A mark made while the runner keeps state is owed to it**, and kept in the
 *   store, not only here, until the runner answers, so a relaunch still owes
 *   it. Any answer settles it, whether the runner applied it or skipped a
 *   ticket that was deleted or moved. A board read never waits for it.
 * - **What this phone kept before this launch goes up once per runner: its
 *   marks, and never a floor.** A phone's floor is a first look nobody saw as
 *   Unread; sent, it would mark everything older than a day read on every
 *   device.
 * - **A phone's floor counts as its own only if somebody set it on this build**
 *   (a Mark All as Read that this phone kept, [BoardReadsStore.floorWasSet]). One
 *   saved before that, by an older build or a first look, is made up: never sent,
 *   and the runner's state replaces it.
 * - **A grant without Control can't write the runner's state** ([mayWrite]): its
 *   marks are kept here, merged by max with the runner's, and never sent.
 * - **A runner that sends no state is an older build**: marks go to the store,
 *   times from this phone's clock, as they always did.
 *
 * Not thread-safe: every caller is on the main dispatcher, as [scope] is.
 */
class BoardReadsKeeper(
    private val store: BoardReadsStore,
    private val host: String,
    private val workspace: String,
    nowMs: Long,
    private val scope: CoroutineScope,
    /** Whether this phone's grant lets it write the runner's read state (Control). */
    private val mayWrite: () -> Boolean = { true },
    /** Tell the runner a raise: its answer, or null when it gave none. */
    private val send: suspend (ReadsRaise) -> String?,
) {
    private val beforeLaunch: BoardReads? = store.keptReads(host, workspace)
    private var pending: ReadsRaise = store.loadPending(host, workspace)
    private val _reads = MutableStateFlow(pending.applied(store.load(host, workspace, nowMs)))

    /** What's read, as this phone shows it now. */
    val reads: StateFlow<BoardReads> = _reads.asStateFlow()

    /** Whether the runner keeps this board's state: it sent it with the board. */
    var runnerKeepsReads = false
        private set

    private var runnerReads: BoardReads? = null

    /** Whether this phone's floor was somebody's doing (a Mark All as Read) and not the first look `load` made up. */
    private val keptARealFloor: Boolean
        get() = (beforeLaunch?.floorMs ?: Long.MIN_VALUE) > Long.MIN_VALUE && store.floorWasSet(host, workspace)

    /** Whether marking something read here reaches every device: the runner keeps this board's state and this phone may write it. */
    val readsAreShared: Boolean get() = runnerKeepsReads && mayWrite()
    private val sending = Mutex()

    /**
     * A board read's text: its `reads` adopted when the runner sent any, this
     * phone's own marks sent up once, and what's owed sent behind the read. A
     * runner that sends none after it did went back to an older build: this
     * phone keeps the state again.
     */
    fun adopt(board: String) {
        val wire = WireBoardReads.ofBoard(board)
        if (wire != null) {
            adopt(wire.reads)
            seedFromThisPhone()
            queueFlush()
        } else if (runnerKeepsReads) {
            runnerKeepsReads = false
            runnerReads = null
            store.save(_reads.value, host, workspace)
        }
    }

    /** Another device read something: the runner's `reads` event. */
    fun heard(wire: WireBoardReads) = adopt(wire.reads)

    /** [row] was opened: everything on it so far is read, its notes through [latestMs]. */
    fun open(row: TaskRow, latestMs: Long? = null, nowMs: Long = System.currentTimeMillis()) {
        val now = _reads.value
        val next = if (runnerKeepsReads) now.openSeenThrough(row, latestMs) else now.openSeenThrough(row, maxOf(nowMs, latestMs ?: Long.MIN_VALUE))
        if (next == now) return
        _reads.value = next
        keep(ReadsRaise(opened = next.opened[row.id]?.let { mapOf(row.id to it) } ?: emptyMap()))
    }

    /** Mark All as Read: everything on [rows] so far. On a runner that keeps it, that clears every device. */
    fun markAllRead(rows: List<TaskRow>, latestMs: Long? = null, nowMs: Long = System.currentTimeMillis()) {
        val now = _reads.value
        _reads.value = if (runnerKeepsReads) now.markAllReadSeenThrough(rows, latestMs)
        else now.markAllReadOnDeviceClock(rows, nowMs)
        if (!readsAreShared) store.markFloorSet(host, workspace)
        keep(ReadsRaise(floorMs = _reads.value.floorMs))
    }

    /** Send the runner what it's owed, and wait for it, one send after another. */
    suspend fun flush() = queueFlush().join()

    private fun adopt(state: BoardReads) {
        if (!runnerKeepsReads && !keptARealFloor) {
            // The floor this phone made up on its first look hid nothing from
            // the runner and must not hide what the runner shows: the runner's
            // state stands in for it, with the marks this phone has.
            _reads.value = pending.applied(BoardReads(Long.MIN_VALUE, _reads.value.opened))
        }
        runnerKeepsReads = true
        val known = runnerReads?.merged(state) ?: state
        runnerReads = known
        val merged = _reads.value.merged(known)
        if (merged != _reads.value) _reads.value = merged
    }

    /** A change to the state: owed to a runner that keeps it, until it answers; saved here for one that can't. */
    private fun keep(change: ReadsRaise) {
        if (readsAreShared) {
            pending = pending.merging(change)
            store.savePending(pending, host, workspace)
            queueFlush()
        } else {
            store.save(_reads.value, host, workspace)
        }
    }

    /**
     * The upgrade, once per runner: the marks this phone kept before this
     * launch are owed to the runner, which merges by max. Nothing from a phone
     * that kept none, and never a floor.
     */
    private fun seedFromThisPhone() {
        if (!mayWrite() || store.isUploaded(host, workspace)) return
        beforeLaunch?.takeIf { it.opened.isNotEmpty() }?.let {
            pending = pending.merging(ReadsRaise(opened = it.opened))
            store.savePending(pending, host, workspace)
        }
        store.markUploaded(host, workspace)
    }

    private fun queueFlush(): Job = scope.launch { sending.withLock { sendPending() } }

    private suspend fun sendPending() {
        if (!readsAreShared || pending.isEmpty) return
        val sent = pending
        val answer = send(sent)?.let(WireBoardReads::ofState) ?: return
        pending = pending.without(sent)
        store.savePending(pending, host, workspace)
        adopt(answer.reads)
    }
}
