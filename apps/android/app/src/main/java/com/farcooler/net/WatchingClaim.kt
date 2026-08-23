package com.farcooler.net

/**
 * When to tell a runner which panes are in front of somebody.
 *
 * The rate limiter behind `Connection.reportWatching`, kept as its own type
 * because it is the only part of that path with a decision in it — everything
 * else is a round trip. The claim is a HEARTBEAT: the runner believes one for
 * ten seconds (`WATCHED_TTL_MS` in `crates/daemon/src/watch.rs`) and the poll
 * renews it every three, so without a floor a person swiping through a tab strip
 * would spend one SSH round trip per frame.
 *
 * Two grounds for sending, and they are different questions. The set CHANGED —
 * which must go immediately, because backing out of a pane has to release it in
 * the same round trip rather than leaving it silent until the claim ages out.
 * Or the floor has passed — which is the renewal, and is what stops a pane
 * somebody has been reading for a minute from starting to buzz them.
 *
 * Recorded on the ATTEMPT rather than on success. A runner that refuses this in
 * a millisecond must not thereby be asked a thousand times a second; the floor
 * is a floor on asking.
 */
class WatchingClaim(private val floorMs: Long = FLOOR_MS) {

    /**
     * What was last claimed, and when. Null rather than an empty list for
     * "nothing has been claimed on this link yet", which is a different state
     * from "claiming nothing": the first must send, so that a runner that just
     * came up hears from this client at all.
     */
    private var last: List<String>? = null
    private var sentAt = 0L

    /**
     * Whether to send [terminals] now, recording the attempt if so.
     *
     * [nowMs] is the caller's clock and deliberately a parameter: the caller
     * passes `SystemClock.elapsedRealtime()`, which counts since boot and is
     * immune to the clock being stepped — an NTP correction against a wall clock
     * would read as a claim made in the future, and the daemon treats one of
     * those as expired.
     */
    fun due(terminals: List<String>, nowMs: Long): Boolean {
        val changed = terminals != last
        if (!changed && nowMs - sentAt < floorMs) return false
        last = terminals
        sentAt = nowMs
        return true
    }

    /**
     * Forget what this link was told, because it is a different link now.
     *
     * A claim is state on the RUNNER, and a reconnect is a runner that has been
     * told nothing. Without this, a phone that dropped and came back while
     * sitting on one pane would match its own last claim, skip the send, and
     * stay silent for as long as that pane stayed on screen — the exact case
     * where the claim is worth most.
     */
    fun reset() {
        last = null
        sentAt = 0L
    }

    companion object {
        /**
         * The shortest gap between two identical claims.
         *
         * Two seconds, matching iOS's `watchingFloor`, under the three-second
         * poll that renews them — so the poll sets the real cadence and this
         * only absorbs the extra calls a person generates by moving between
         * tabs. Well under the runner's ten-second TTL, which leaves room for
         * two lost round trips before a pane somebody is plainly looking at
         * starts buzzing them again.
         */
        const val FLOOR_MS = 2_000L
    }
}
