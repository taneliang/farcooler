package com.farcooler.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.time.temporal.ChronoUnit
import java.util.Locale

// When a card starts, who is working it, and the sentences that say so
// (ov-212, ov-213).
//
// The Android twin of AgentKit's `TaskStartLines.swift`, which the Mac and the
// iPhone draw from. Both are checked against ONE fixture,
// `task_start_lines.json`, so a card reads the same on all three. The runner
// sends stable words (`in_line`, `release`, `finished`); every sentence a
// person reads is made here. Sentence case, because it is body text.
//
// Ordinals and spans are written out and not taken from a formatter, as
// `TaskRow.ago` is: a sentence that changed under another locale would be one
// the tests say nothing about. The two clock times are the exception, and are
// the user's locale's.

/** Which of the board's two lines a waiting task is standing in. */
@Serializable
enum class TaskLine(val wire: String) {
    @SerialName("agent") AGENT("agent"),
    @SerialName("build") BUILD("build"),
}

/** What a task held for an event is waiting for. */
@Serializable
enum class TaskWaitEvent(val wire: String) {
    @SerialName("release") RELEASE("release"),
    @SerialName("recurrence") RECURRENCE("recurrence"),
    @SerialName("clear_board") CLEAR_BOARD("clear_board"),
}

/**
 * What a task is waiting for before it starts. A word this build does not
 * know is no wait at all (null on the row), never a guess.
 */
@Serializable
sealed interface TaskWait {
    /** [position] 1 is next; 0 is on the line with no rank yet. */
    @Serializable
    @SerialName("in_line")
    data class InLine(val line: TaskLine, val position: Int) : TaskWait

    @Serializable
    @SerialName("until")
    data class Until(@SerialName("until") val atMs: Long) : TaskWait

    @Serializable
    @SerialName("after")
    data class After(val event: TaskWaitEvent) : TaskWait

    @Serializable
    @SerialName("parked")
    data object Parked : TaskWait
}

/** What the runner knows of one subagent working a task. */
@Serializable
enum class TaskWorkerState(val wire: String) {
    @SerialName("running") RUNNING("running"),

    /** Open, and the runner can't see it: only what was recorded. */
    @SerialName("unobserved") UNOBSERVED("unobserved"),
    @SerialName("finished") FINISHED("finished"),

    /** Failed, killed or stopped. */
    @SerialName("stopped") STOPPED("stopped");

    /** Still open: the orchestrator has not closed it out. */
    val isOpen: Boolean get() = this == RUNNING || this == UNOBSERVED
}

/** One subagent working a task from inside another agent's session. */
@Serializable
data class TaskWorker(
    val harness: String,
    val state: TaskWorkerState,
    @SerialName("started_at") val startedAtMs: Long? = null,
    @SerialName("ended_at") val endedAtMs: Long? = null,
    /** What it is doing right now, in the runner's words, or empty. */
    val doing: String = "",
    /** The pane its orchestrator runs in: where the subagent lives. */
    @SerialName("orchestrator_terminal") val orchestratorTerminalId: String? = null,
    /** Its model, as the runner recorded it (`claude-opus-5-5`), or empty. */
    val model: String = "",
)

/** How the subagent control reads: some still at it, or the last one done. */
@Serializable
enum class SubagentState {
    @SerialName("working") WORKING,
    @SerialName("finished") FINISHED,
    @SerialName("stopped") STOPPED,
}

internal fun TaskRow.workerFields() = buildJsonObject {
    put("workers", buildJsonArray {
        workers.forEach { worker ->
            add(buildJsonObject {
                put("state", worker.state.wire)
                worker.endedAtMs?.let { put("ended", it) }
                put("orchestrator", worker.orchestratorTerminalId)
            })
        }
    })
    put("missing_stamp", Long.MIN_VALUE)
}

private fun TaskRow.workerView(): TaskBoardPresenter.Workers =
    TaskBoardPresenter.ask("workers", workerFields())

val TaskRow.openWorkers: List<TaskWorker> get() = workerView().open.map(workers::get)
val TaskRow.orchestratorTerminalId: String? get() = workerView().orchestrator

/**
 * Whether a block holds a task in this status back. A task that is done,
 * canceled, in review or waiting on a decision is not held by one.
 */
fun holdsBlocks(status: TaskStatus): Boolean =
    TaskBoardPresenter.byWire.getValue(status.wire).holdsBlocks

/** `a`, `a and b`, `a, b and c`: AgentKit's `TaskRow.listed`. */
private fun listed(items: List<String>): String = when (items.size) {
    0 -> ""
    1 -> items[0]
    2 -> "${items[0]} and ${items[1]}"
    else -> items.dropLast(1).joinToString(", ") + " and " + items.last()
}

/** What the card is waiting on, in one line, or null: `Waiting on ov-191 and ov-192`. */
val TaskRow.blockedSummary: String?
    get() = if (blockedBy.isEmpty()) null else "Waiting on " + listed(blockedBy)

/**
 * The card's quiet second line about when it starts and who is on it, or null.
 * Two parts: why it has not started or is waiting (the runner's wait, shown
 * only in the statuses where it means something, so an older runner that moved
 * a task without clearing its wait cannot put a stale line on screen), then
 * the subagent. A running subagent is the explanation, so it speaks only when
 * there is no wait; a finished one speaks beside it ("Waiting to build, 2nd in
 * line · Subagent finished 12 min ago").
 *
 * [speaksOfAgents] is [TaskAgentLink.speaksOfAgents]: false when the runner
 * cannot be believed about its agents right now, which drops the subagent part
 * and leaves the board's own wait.
 */
fun TaskRow.startLine(
    nowMs: Long,
    speaksOfAgents: Boolean = true,
    zone: ZoneId = ZoneId.systemDefault(),
    locale: Locale = Locale.getDefault(),
): String? {
    val waitPart = waitSentence(nowMs, zone, locale)
    val parts = mutableListOf<String>()
    if (waitPart != null) parts += waitPart
    if (speaksOfAgents && status == TaskStatus.IN_PROGRESS && workers.isNotEmpty()) {
        val running = openWorkers.isNotEmpty()
        if (!(running && waitPart != null)) workerSentence(nowMs)?.let { parts += it }
    }
    if (parts.isEmpty() && status == TaskStatus.TODO && blockedBy.isEmpty()) return "Ready to start"
    return if (parts.isEmpty()) null else parts.joinToString(" · ")
}

private fun TaskRow.waitSentence(nowMs: Long, zone: ZoneId, locale: Locale): String? {
    val w = wait ?: return null
    return when (w) {
        is TaskWait.InLine -> when (w.line) {
            TaskLine.AGENT -> {
                if ((status != TaskStatus.BACKLOG && status != TaskStatus.TODO) || w.position <= 0) null
                else if (w.position == 1) "Next to start" else "${ordinal(w.position)} in line"
            }
            TaskLine.BUILD -> {
                if ((status != TaskStatus.IN_PROGRESS && status != TaskStatus.IN_REVIEW) || w.position <= 0) null
                else if (w.position == 1) "Builds next" else "Waiting to build, ${ordinal(w.position)} in line"
            }
        }
        // A time that has come reads as an ordinary Backlog card: the runner
        // clears it a moment later, and "Starts at 9:00 AM" in the past is the
        // one wrong thing to say meanwhile.
        is TaskWait.Until ->
            if (status != TaskStatus.BACKLOG || w.atMs <= nowMs) null
            else "Starts " + whenWords(w.atMs, nowMs, zone, locale)
        is TaskWait.After -> if (status != TaskStatus.BACKLOG) null else when (w.event) {
            TaskWaitEvent.RELEASE -> "Starts after the next release"
            TaskWaitEvent.RECURRENCE -> "Starts if it happens again"
            TaskWaitEvent.CLEAR_BOARD -> "Starts when nothing else is waiting"
        }
        TaskWait.Parked -> if (status == TaskStatus.BACKLOG) "Not planned" else null
    }
}

private fun harnessWord(harness: String): String = when (harness) {
    "claude" -> "Claude"
    "codex" -> "Codex"
    else -> ""
}

private fun TaskRow.workerSentence(nowMs: Long): String? {
    val view = workerView()
    val open = view.open.map(workers::get)
    if (open.size > 1) return "${open.size} subagents working"
    val worker = open.firstOrNull()
    if (worker != null) {
        val who = harnessWord(worker.harness)
        val head = if (who.isEmpty()) "Subagent" else "$who subagent"
        val observed = worker.state == TaskWorkerState.RUNNING && who != "Codex"
        val started = worker.startedAtMs
        if (!observed) return if (started == null) head else "$head, started ${agoWords(started, nowMs)}"
        var line = "$head working"
        if (started != null) line += ", ${spanWords(started, nowMs)}"
        if (worker.doing.isNotEmpty()) line += " · ${worker.doing}"
        return line
    }
    val last = view.last?.let(workers::get) ?: return null
    val verb = if (last.state == TaskWorkerState.STOPPED) "stopped" else "finished"
    val ended = last.endedAtMs ?: return "Subagent $verb"
    return "Subagent $verb ${agoWords(ended, nowMs)}"
}

/** `1st`, `2nd`, `3rd`, `4th`, `11th`, `12th`, `21st`. */
internal fun ordinal(n: Int): String {
    if (n % 100 in 11..13) return "${n}th"
    return when (n % 10) {
        1 -> "${n}st"
        2 -> "${n}nd"
        3 -> "${n}rd"
        else -> "${n}th"
    }
}

private fun minutesBetween(thenMs: Long, nowMs: Long): Long = maxOf(0L, nowMs - thenMs) / 60_000L

private fun spanMinutes(minutes: Long): String = when {
    minutes < 60 -> "$minutes min"
    minutes < 24 * 60 -> "${minutes / 60} h"
    else -> "${minutes / (24 * 60)} d"
}

/** `12 min ago`, `3 h ago`, `2 d ago`, or `just now` under a minute. */
internal fun agoWords(thenMs: Long, nowMs: Long): String {
    val minutes = minutesBetween(thenMs, nowMs)
    return if (minutes < 1) "just now" else spanMinutes(minutes) + " ago"
}

/** `12 min`, `3 h`, `2 d`, or `under 1 min`. */
internal fun spanWords(thenMs: Long, nowMs: Long): String {
    val minutes = minutesBetween(thenMs, nowMs)
    return if (minutes < 1) "under 1 min" else spanMinutes(minutes)
}

/**
 * `at 9:00 AM`, `tomorrow at 9:00 AM`, or `Mon, Oct 5 at 9:00 AM`, in the given
 * zone and locale. The narrow no-break space newer ICU puts before AM is a
 * plain space, so the same words come out wherever they are made.
 */
internal fun whenWords(atMs: Long, nowMs: Long, zone: ZoneId, locale: Locale): String {
    val at = Instant.ofEpochMilli(atMs).atZone(zone)
    val clock = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale)
        .format(at).replace(' ', ' ')
    val days = ChronoUnit.DAYS.between(
        Instant.ofEpochMilli(nowMs).atZone(zone).toLocalDate(), at.toLocalDate(),
    )
    return when {
        days < 1 -> "at $clock"
        days == 1L -> "tomorrow at $clock"
        else -> DateTimeFormatter.ofPattern("EEE, MMM d", locale).format(at) + " at $clock"
    }
}
