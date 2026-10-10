package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

// Trains (ov-309) and the CI the runner reads (ov-306), read with the plan
// (`plan.get`'s `trains` and `ci`, the shape the CLI's `plan --json` prints
// and `test/fixtures/plan.json` holds). The Android twin of AgentKit's
// `PlanTrains.swift`. EXPERIMENTAL with the plan layer, behind the runner's
// `board_trains` capability. Copy is Material's: sentence case.

/** Where a train stands. */
enum class TrainState(val wire: String) {
    INTEGRATING("integrating"),
    GATING("gating"),
    PUSHED("pushed"),
    GREEN("green"),
    RED("red"),
    LANDED("landed"),
    DROPPED("dropped"),
    UNKNOWN("unknown");

    /** Landed or dropped: out of Now, and its CI no longer read. */
    val isSettled: Boolean get() = this == LANDED || this == DROPPED

    companion object {
        fun parse(word: String?): TrainState = entries.firstOrNull { it.wire == word } ?: UNKNOWN
    }
}

/** A lane on a train, by id and name. */
data class PlanTrainLane(val lane: String, val name: String)

data class PlanTrain(
    val id: String,
    val name: String,
    /** What it was cut from: `origin/main`, or a SHA. */
    val base: String,
    val pushedSha: String?,
    val state: TrainState,
    val stateSince: Long,
    val lanes: List<PlanTrainLane>,
    /** The CI subject its pushed SHA is read under; empty before it has one. */
    val ciSubject: String,
    /** What a person reads for it (ov-462): the title the orchestrator wrote, else "Train 72". Null from a runner before it. */
    val title: String? = null,
    /** One line of what it carries: its lanes' titles (ov-462). */
    val summary: String? = null,
    /** The card its integrating agent works for it (ov-461). */
    val card: PlanCardRef? = null,
    /** The agent integrating it, with its state and spend (ov-461). */
    val agent: PlanTrainAgent? = null,
) {
    /** What every surface shows first: the title, else "Train 72" read off the name, else the name. */
    val heading: String get() = title?.takeIf { it.isNotEmpty() } ?: TrainWords.number(name)?.let { "Train $it" } ?: name

    /** The slug, shown second where there is room: null when the heading is the name already. */
    val slug: String? get() = name.takeIf { heading != it }

    /** Its lanes' titles on one line, or null when it carries none. */
    val carries: String? get() = summary?.takeIf { it.isNotEmpty() }

    companion object {
        fun decodeAll(plan: JsonObject): List<PlanTrain> = plan.items("trains").map {
            val o = it.jsonObject
            PlanTrain(
                id = o.text("id"), name = o.text("name"), base = o.text("base"), pushedSha = o.maybeText("pushed_sha"),
                state = TrainState.parse(o.maybeText("state")), stateSince = o["state_since"]?.jsonPrimitive?.longOrNull ?: 0L,
                lanes = o.items("lanes").map { l -> l.jsonObject.let { PlanTrainLane(it.text("lane"), it.text("name")) } },
                ciSubject = o.text("ci_subject"), title = o.maybeText("title"), summary = o.maybeText("summary"),
                card = (o["card"] as? JsonObject)?.let { PlanCardRef(it.text("task"), it.text("key")) },
                agent = (o["agent"] as? JsonObject)?.let { a ->
                    PlanTrainAgent(
                        harness = a.text("harness"), agentId = a.text("agent_id"), model = a.text("model"),
                        startedAt = a["started_at"]?.jsonPrimitive?.longOrNull ?: 0L,
                        endedAt = a["ended_at"]?.jsonPrimitive?.longOrNull,
                        spend = Plan.spend(a["spend"] as? JsonObject),
                    )
                },
            )
        }
    }
}

/** The agent integrating a train (ov-461): its state and what it has spent. Never a lane of the train's own name. */
data class PlanTrainAgent(
    val harness: String,
    val agentId: String,
    val model: String,
    val startedAt: Long,
    val endedAt: Long?,
    val spend: PlanSpend,
) {
    val isWorking: Boolean get() = endedAt == null
}

/** Where a CI subject stands, over all its runs. */
enum class CiStatus(val wire: String) {
    PASSED("passed"),
    FAILED("failed"),
    RUNNING("running"),
    QUEUED("queued"),
    /** Nothing failed and a run was canceled, as CI does when a newer push supersedes it: neutral, never red. */
    SUPERSEDED("superseded"),
    NONE("none"),
    UNKNOWN("unknown");

    companion object {
        fun parse(word: String?): CiStatus = entries.firstOrNull { it.wire == word } ?: UNKNOWN
    }
}

/** One job: "CI / Rust (ubuntu-latest)", and `passed`, `failed`, `running`, `queued`, `skipped` or `canceled`. */
data class PlanCiJob(val name: String, val state: String, val url: String = "")

/** What the runner last read of one subject: `main`, `sha:<sha>` or `run:<id>`. */
data class PlanCiRead(
    val subject: String,
    val sha: String = "",
    val status: CiStatus,
    val url: String = "",
    val jobs: List<PlanCiJob> = emptyList(),
    /** When a read last worked: what's here is as of then. 0 if none has. */
    val fetchedAt: Long = 0,
) {
    /** A failed run needs the owner: the one CI state drawn in color. */
    val needsAttention: Boolean get() = status == CiStatus.FAILED

    companion object {
        fun decodeAll(plan: JsonObject): List<PlanCiRead> = plan.items("ci").map {
            val o = it.jsonObject
            PlanCiRead(
                subject = o.text("subject"), sha = o.text("sha"), status = CiStatus.parse(o.maybeText("status")),
                url = o.text("url"), fetchedAt = o["fetched_at"]?.jsonPrimitive?.longOrNull ?: 0L,
                jobs = o.items("jobs").map { j -> j.jsonObject.let { PlanCiJob(it.text("name"), it.text("state"), it.text("url")) } },
            )
        }
    }
}

/** One group of Now: a train not yet landed and its lanes working now, or the lanes on none. */
data class PlanNowGroup(val train: PlanTrain?, val lanes: List<PlanLane>)

/** The read a subject names: `sha:<sha>` matches a read of the same commit however short either was written. */
fun Plan.ci(subject: String): PlanCiRead? {
    val wanted = subject.lowercase()
    ci.firstOrNull { it.subject == wanted }?.let { return it }
    if (!wanted.startsWith("sha:")) return null
    val sha = wanted.removePrefix("sha:")
    return ci.firstOrNull { read ->
        val theirs = if (read.subject.startsWith("sha:")) read.subject.removePrefix("sha:") else ""
        (theirs.isNotEmpty() && (theirs.startsWith(sha) || sha.startsWith(theirs))) || (read.sha.isNotEmpty() && read.sha.startsWith(sha))
    }
}

/** A train's CI, once the runner has read it. */
fun Plan.ciOf(train: PlanTrain): PlanCiRead? = if (train.ciSubject.isEmpty()) null else ci(train.ciSubject)

/** Now as row groups (ov-309): each live train heading the lanes on it, then the lanes on none. */
val Plan.nowGroups: List<PlanNowGroup>
    get() {
        // A lane named like a live train is the train's agent, drawn by the train (ov-461).
        val shadows = laneIsTrain.map { it.lane }.toSet()
        val lanes = (working + unranked).filter { it.id !in shadows }
        val grouped = mutableSetOf<String>()
        val groups = trains.filter { !it.state.isSettled }.map { train ->
            val ids = train.lanes.map { it.lane }.toSet()
            val mine = lanes.filter { it.id in ids }
            grouped += mine.map { it.id }
            PlanNowGroup(train, mine)
        }
        val rest = lanes.filter { it.id !in grouped }
        return if (rest.isEmpty()) groups else groups + PlanNowGroup(null, rest)
    }

/** How many of the board's cards are in [status] (a board status's word, or `open`); null without the counts. */
fun Plan.cardCount(status: String): Int? {
    val c = boardCounts ?: return null
    return when (status) {
        "backlog" -> c.backlog
        "todo" -> c.todo
        "needs_decision" -> c.needsDecision
        "in_progress" -> c.inProgress
        "in_review" -> c.inReview
        "done" -> c.done
        "cancelled" -> c.cancelled
        "open" -> c.backlog + c.todo + c.needsDecision + c.inProgress + c.inReview
        else -> null
    }
}

/** The words for trains and CI, in Material's sentence case. */
object TrainWords {
    fun state(state: TrainState): String = when (state) {
        TrainState.INTEGRATING -> "Integrating"
        TrainState.GATING -> "Gating"
        TrainState.PUSHED -> "Pushed"
        TrainState.GREEN -> "Green"
        TrainState.RED -> "Red"
        TrainState.LANDED -> "Landed"
        TrainState.DROPPED -> "Dropped"
        TrainState.UNKNOWN -> "Unknown"
    }

    fun ciStatus(status: CiStatus): String = when (status) {
        CiStatus.PASSED -> "Passed"
        CiStatus.FAILED -> "Failed"
        CiStatus.RUNNING -> "Running"
        CiStatus.QUEUED -> "Queued"
        CiStatus.SUPERSEDED -> "Superseded"
        CiStatus.NONE -> "No runs yet"
        CiStatus.UNKNOWN -> "CI unknown"
    }

    /** "1 of 3 jobs failed", "2 of 3 jobs done", "4 jobs"; null with none read. */
    fun ciJobs(read: PlanCiRead): String? {
        val total = read.jobs.size
        if (total == 0) return null
        fun n(state: String) = read.jobs.count { it.state == state }
        val jobs = if (total == 1) "job" else "jobs"
        return when (read.status) {
            // A canceled job is superseded, not failed (review train-1005c H1).
            CiStatus.FAILED -> "${n("failed")} of $total $jobs failed"
            CiStatus.RUNNING, CiStatus.QUEUED -> "${total - n("running") - n("queued")} of $total $jobs done"
            else -> "$total $jobs"
        }
    }

    /** Older than this a read is stale: GitHub hasn't answered (review train-1005c M1); the CLI and AgentKit hold the same. */
    const val CI_STALE_AFTER_MS = 25 * 60_000L

    /** "as of 3 h ago" for a read that worked once and not lately; null while current or never read. */
    fun ciStale(read: PlanCiRead, now: Long): String? =
        if (read.fetchedAt > 0 && now - read.fetchedAt > CI_STALE_AFTER_MS) "as of ${PlanWords.ago(read.fetchedAt, now)}" else null

    /** "Failed · 1 of 3 jobs failed". */
    fun ciSummary(read: PlanCiRead): String = listOfNotNull(ciStatus(read.status), ciJobs(read)).joinToString(" · ")

    /** "Red · c85bf83d · CI Failed · 1 of 3 jobs failed", or "CI not read yet" once pushed. */
    fun train(train: PlanTrain, ci: PlanCiRead?, now: Long = 0): String {
        val parts = mutableListOf(state(train.state))
        train.pushedSha?.takeIf { it.isNotEmpty() }?.let { parts += it.take(8) }
        if (ci != null) {
            // "CI unknown" already says CI (review train-1005c L2).
            val said = ciSummary(ci)
            parts += if (said.startsWith("CI ")) said else "CI $said"
            ciStale(ci, now)?.let { parts += it }
        }
        if (ci == null && train.pushedSha != null && !train.state.isSettled) parts += "CI not read yet"
        train.agent?.let { parts += agent(it) }
        return parts.joinToString(" · ")
    }

    /** The integrating agent's state and spend: "Agent working · 120K tokens", "Agent finished". */
    fun agent(agent: PlanTrainAgent): String {
        val state = if (agent.isWorking) "Agent working" else "Agent finished"
        if (agent.spend.runs == 0 && agent.spend.unmeasuredAgents == 0) return state
        return "$state · ${PlanWords.spend(agent.spend)}"
    }

    /** The number in `train-72` or `integ-72`: what a person calls the train. Null for any other name. */
    fun number(name: String): Int? {
        val lower = name.trim().lowercase()
        val digits = listOf("train-", "integ-").firstOrNull { lower.startsWith(it) }?.let { lower.removePrefix(it) } ?: return null
        return if (digits.isEmpty() || digits.length > 9 || !digits.all { it in '0'..'9' }) null else digits.toInt()
    }

    /** Red, or its CI failed. */
    fun needsAttention(train: PlanTrain, ci: PlanCiRead?): Boolean = train.state == TrainState.RED || ci?.needsAttention == true
}

private fun JsonObject.text(name: String): String = maybeText(name) ?: ""
private fun JsonObject.maybeText(name: String): String? =
    (this[name] as? JsonElement)?.takeIf { it !is JsonNull }?.jsonPrimitive?.contentOrNull
private fun JsonObject.items(name: String): JsonArray = this[name] as? JsonArray ?: JsonArray(emptyList())
