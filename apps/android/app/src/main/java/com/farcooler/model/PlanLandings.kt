package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

// Landings (ov-309, ov-872) and the CI the runner reads (ov-306), read with the plan
// (`plan.get`'s `landings` and `ci`, the shape the CLI's `plan --json` prints
// and `test/fixtures/plan.json` holds). The Android twin of AgentKit's
// `PlanLandings.swift`. EXPERIMENTAL with the plan layer. Copy is Material's:
// sentence case.

/** Where a landing stands. */
enum class LandingState(val wire: String) {
    OPEN("open"),
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
        fun parse(word: String?): LandingState = entries.firstOrNull { it.wire == word } ?: UNKNOWN
    }
}

/** How a landing reaches main. */
enum class LandingRoute(val wire: String) {
    PUSH("push"), PULL_REQUEST("pull_request"), ASK("ask"), UNKNOWN("unknown");

    companion object {
        fun parse(word: String?): LandingRoute = entries.firstOrNull { it.wire == word } ?: UNKNOWN
    }
}

/** Tickets that land on main together, after one round of checks. */
data class PlanLanding(
    val id: String,
    /** Its number: "Landing 9". */
    val number: Int,
    /** What it was cut from: `origin/main`, or a SHA. */
    val base: String,
    val pushedSha: String?,
    val state: LandingState,
    val stateSince: Long,
    val route: LandingRoute = LandingRoute.UNKNOWN,
    /** The tickets it carries, by their cards. */
    val tickets: List<PlanCardRef>,
    /** The CI subject its pushed SHA is read under; empty before it has one. */
    val ciSubject: String,
    /** What a person reads for it (ov-462): the title the orchestrator wrote, else "Landing 72". */
    val title: String? = null,
    /** One line of what it carries: its tickets' titles (ov-462). */
    val summary: String? = null,
    /** The agent landing it, by id (ov-872); null with none. Look it up in [Plan.agents]. */
    val lander: String? = null,
    /** Gathering ready tickets while the landing ahead finishes. */
    val collecting: Boolean = false,
    /** The phase the runner derived (ov-503); null from a runner without it, and for phase 0. */
    val phase: LandingPhase? = null,
    /** When the landing entered [phase], in ms; null with it. */
    val phaseSince: Long? = null,
    /** What it spent, its lander's included (ov-872); null from a runner without it. */
    val spend: PlanSpend? = null,
) {
    /** "Landing 72": what every surface calls the landing, as its heading and in a sentence (ov-514, ov-522). The title is its subtitle, never its name. */
    val label: String get() = label(number)

    /** What it is for, under the label: the title without a "Landing 72 ·" the runner put in front, or null when it has none or says no more than the label. */
    val subtitle: String?
        get() {
            var t = title?.takeIf { it.isNotEmpty() && it != label } ?: return null
            if (t.startsWith("$label · ")) t = t.drop(label.length + 3)
            return t.takeIf { it.isNotEmpty() }
        }

    /** Its tickets' titles on one line, or null when it carries none. */
    val carries: String? get() = summary?.takeIf { it.isNotEmpty() }

    companion object {
        /** "Landing 72". */
        fun label(number: Int): String = "Landing $number"

        fun decodeAll(plan: JsonObject): List<PlanLanding> = plan.items("landings").map {
            val o = it.jsonObject
            PlanLanding(
                id = o.text("id"), number = o["number"]?.jsonPrimitive?.longOrNull?.toInt() ?: 0, base = o.text("base"),
                pushedSha = o.maybeText("pushed_sha"),
                state = LandingState.parse(o.maybeText("state")), stateSince = o["state_since"]?.jsonPrimitive?.longOrNull ?: 0L,
                route = LandingRoute.parse(o.maybeText("route")),
                tickets = o.items("tickets").map { t -> t.jsonObject.let { PlanCardRef(it.text("task"), it.text("key")) } },
                ciSubject = o.text("ci_subject"), title = o.maybeText("title"), summary = o.maybeText("summary"),
                lander = o.maybeText("lander"), collecting = o["collecting"]?.jsonPrimitive?.contentOrNull == "true",
                phase = o.maybeText("phase")?.let(LandingPhase::parse),
                phaseSince = o["phase_since"]?.jsonPrimitive?.longOrNull,
                spend = (o["spend"] as? JsonObject)?.let { Plan.spend(it) },
            )
        }
    }
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

/** A landing's CI, once the runner has read it. */
fun Plan.ciOf(landing: PlanLanding): PlanCiRead? = if (landing.ciSubject.isEmpty()) null else ci(landing.ciSubject)

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

/** The words for landings and CI, in Material's sentence case. */
object LandingWords {
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
            // A canceled job is superseded, not failed (review 1005c H1).
            CiStatus.FAILED -> "${n("failed")} of $total $jobs failed"
            CiStatus.RUNNING, CiStatus.QUEUED -> "${total - n("running") - n("queued")} of $total $jobs done"
            else -> "$total $jobs"
        }
    }

    /** Older than this a read is stale: GitHub hasn't answered (review 1005c M1); the CLI and AgentKit hold the same. */
    const val CI_STALE_AFTER_MS = 25 * 60_000L

    /** "as of 3 h ago" for a read that worked once and not lately; null while current or never read. */
    fun ciStale(read: PlanCiRead, now: Long): String? =
        if (read.fetchedAt > 0 && now - read.fetchedAt > CI_STALE_AFTER_MS) "as of ${PlanWords.ago(read.fetchedAt, now)}" else null

    /** "Failed · 1 of 3 jobs failed". */
    fun ciSummary(read: PlanCiRead): String = listOfNotNull(ciStatus(read.status), ciJobs(read)).joinToString(" · ")

    /** The lander's state and spend: "Lander working · 120K tokens", "Lander finished". */
    fun lander(agent: PlanAgent): String {
        val state = if (agent.working) "Lander working" else "Lander finished"
        if (agent.spend.runs == 0 && agent.spend.unmeasuredAgents == 0) return state
        return "$state · ${PlanWords.spend(agent.spend)}"
    }

    /** Red, or its CI failed. */
    fun needsAttention(landing: PlanLanding, ci: PlanCiRead?): Boolean = landing.state == LandingState.RED || ci?.needsAttention == true
}

private fun JsonObject.text(name: String): String = maybeText(name) ?: ""
private fun JsonObject.maybeText(name: String): String? =
    (this[name] as? JsonElement)?.takeIf { it !is JsonNull }?.jsonPrimitive?.contentOrNull
private fun JsonObject.items(name: String): JsonArray = this[name] as? JsonArray ?: JsonArray(emptyList())
