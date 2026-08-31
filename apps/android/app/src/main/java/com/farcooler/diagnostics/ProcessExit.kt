package com.farcooler.diagnostics

import android.app.ActivityManager
import android.app.ApplicationExitInfo
import android.content.Context
import android.util.Log

/**
 * Why this app's last process went away.
 *
 * ## Why this exists at all
 *
 * `PaneDeck.MOUNT_LIMIT` went from three to five, and the owner's reasoning was
 * to raise it and **watch for real memory pressure rather than design around a
 * predicted one**. That is only a plan if the pressure is observable, and it was
 * not: **a low-memory kill is silent.** Android's low-memory killer takes the
 * process without a crash, without a dialog and without a trace anywhere the app
 * can see; the next launch is a cold start, indistinguishable from a first run
 * or from a crash somebody already fixed. A person reporting "it keeps forgetting
 * where I was" and a person reporting "it restarted" are describing the same
 * event, and nothing in the app could tell them apart.
 *
 * `ActivityManager.getHistoricalProcessExitReasons` is the platform's answer, and
 * it is the whole reason this file is part of the mount-limit decision rather
 * than a follow-up to it. Without it the limit stays at five by DEFAULT rather
 * than by choice, because no evidence would ever arrive to move it.
 *
 * Each record carries the RSS the process was holding when it died, which is
 * exactly the number `PaneDeck`'s budget comment is about — a native alacritty
 * grid per mounted pane, bounded by `SCROLLBACK_LINES` in `crates/vt/src/lib.rs`
 * — so a `REASON_LOW_MEMORY` line says both that it happened and how big we had
 * got.
 *
 * ## Logged, never shown
 *
 * There is no UI for this and there should not be. It is a fact about a process
 * that no longer exists, it is actionable by exactly one audience, and a phone
 * telling somebody at 3am that Android reclaimed it yesterday would be the app
 * spending the person's attention on the app's own problems.
 *
 * ## Every launch, not once per event
 *
 * The whole history is logged on each start rather than being diffed against a
 * high-water mark in preferences. That is deliberate: this is read out of a bug
 * report, and a bug report captures whatever the log holds NOW. An event logged
 * once, on the launch after it happened, is an event that has scrolled out of
 * the buffer by the time anybody asks. Re-stating a short history costs a few
 * lines at startup and means the answer is present whenever the question is.
 */
object ProcessExit {

    private const val TAG = "FarCooler"

    /**
     * How many records to ask for.
     *
     * Enough to show a pattern — three kills in a row is a different report from
     * one kill a week ago — and few enough that this stays a handful of lines at
     * startup. The platform caps its own history at 16.
     */
    private const val HISTORY = 5

    /**
     * Read the history and log it. Call once, at startup.
     *
     * **Off the main thread.** This is a binder round trip to the system server
     * on a path that runs before the first frame, and a cold start is exactly
     * when there is the least budget to spend on something nobody is waiting
     * for. The caller supplies the thread; see `FarCoolerApp`.
     *
     * Failures are swallowed to a log line on purpose. Nothing in the app
     * depends on this answer, and a diagnostic that can take the process down is
     * worse than the silence it was written to end.
     */
    fun report(context: Context, now: Long = System.currentTimeMillis()) {
        val lines = try {
            read(context, now)
        } catch (e: Throwable) {
            Log.w(TAG, "could not read the process exit history", e)
            return
        }
        if (lines.isEmpty()) {
            // Not an error: a first launch after install has no history, and so
            // does a device that has never had to take this process away.
            Log.i(TAG, "process exits: none recorded")
            return
        }
        Log.i(TAG, "process exits, most recent first:")
        lines.forEach { Log.i(TAG, "  $it") }
    }

    private fun read(context: Context, now: Long): List<String> {
        val manager = context.getSystemService(ActivityManager::class.java) ?: return emptyList()
        return manager
            .getHistoricalProcessExitReasons(context.packageName, /* pid = */ 0, HISTORY)
            .map {
                describe(
                    reason = it.reason,
                    status = it.status,
                    rssKb = it.rss,
                    timestamp = it.timestamp,
                    now = now,
                    description = it.description,
                )
            }
    }

    /**
     * One record, in one line.
     *
     * Split out from [read] and given plain parameters so it can be tested on
     * the JVM: `ApplicationExitInfo` cannot be constructed outside the system
     * server, so a mapping written inline in [read] would be a mapping nothing
     * could check — and the one branch that matters, `REASON_LOW_MEMORY`, is the
     * one that fires least often and would be discovered wrong at exactly the
     * wrong moment.
     *
     * @param rssKb the resident set size at death, in kilobytes. Printed in
     *   megabytes because that is the unit the budget is argued in, and because
     *   "312 MB" is a number a person can hold against "five mounted panes"
     *   where "319488" is not.
     */
    fun describe(
        reason: Int,
        status: Int,
        rssKb: Long,
        timestamp: Long,
        now: Long,
        description: String? = null,
    ): String {
        val age = ago(now - timestamp)
        val rss = if (rssKb > 0) ", ${rssKb / 1024} MB resident" else ""
        val extra = description?.takeIf { it.isNotBlank() }?.let { " — $it" } ?: ""
        val exit = if (reason == ApplicationExitInfo.REASON_SIGNALED ||
            reason == ApplicationExitInfo.REASON_EXIT_SELF
        ) " ($status)" else ""
        return "$age: ${word(reason)}$exit$rss$extra"
    }

    /**
     * The reason, in words.
     *
     * **`REASON_LOW_MEMORY` is the one this file was written for** and it is
     * spelled out rather than abbreviated, because it is what somebody will be
     * grepping a bug report for. The others are here because a history that
     * showed only low-memory kills would leave a reader unable to tell "no kills"
     * from "this code does not report anything else", which is the same
     * unobservability one level up.
     *
     * An unrecognised reason prints its number rather than falling into "other":
     * a constant added by a future Android is a fact we have, and flattening it
     * into a catch-all would throw away the only part of it that identifies it.
     */
    fun word(reason: Int): String = when (reason) {
        // The one the mount limit is waiting on.
        ApplicationExitInfo.REASON_LOW_MEMORY -> "REASON_LOW_MEMORY (Android reclaimed us)"
        ApplicationExitInfo.REASON_CRASH -> "crash (unhandled exception)"
        ApplicationExitInfo.REASON_CRASH_NATIVE -> "native crash"
        ApplicationExitInfo.REASON_ANR -> "ANR"
        ApplicationExitInfo.REASON_SIGNALED -> "signalled"
        ApplicationExitInfo.REASON_EXIT_SELF -> "exited on its own"
        ApplicationExitInfo.REASON_USER_REQUESTED -> "the person closed it"
        ApplicationExitInfo.REASON_USER_STOPPED -> "the user was stopped"
        ApplicationExitInfo.REASON_DEPENDENCY_DIED -> "a dependency died"
        ApplicationExitInfo.REASON_INITIALIZATION_FAILURE -> "failed to initialize"
        ApplicationExitInfo.REASON_PERMISSION_CHANGE -> "a permission changed"
        ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE -> "excessive resource usage"
        ApplicationExitInfo.REASON_PACKAGE_UPDATED -> "the package was updated"
        ApplicationExitInfo.REASON_PACKAGE_STATE_CHANGE -> "the package state changed"
        ApplicationExitInfo.REASON_FREEZER -> "frozen"
        ApplicationExitInfo.REASON_OTHER -> "other"
        ApplicationExitInfo.REASON_UNKNOWN -> "unknown"
        else -> "reason $reason"
    }

    /**
     * How long ago, coarsely.
     *
     * The same shape as `GlanceAge` and deliberately NOT that function: this is
     * a log line rather than a glance surface, it is allowed to say "3d", and
     * tying a diagnostic's format to a design spec would mean a change to one
     * silently rewriting the other.
     */
    private fun ago(millis: Long): String {
        val seconds = (if (millis < 0) 0 else millis) / 1000
        return when {
            seconds < 60 -> "just now"
            seconds < 3600 -> "${seconds / 60}m ago"
            seconds < 86_400 -> "${seconds / 3600}h ago"
            else -> "${seconds / 86_400}d ago"
        }
    }
}
