package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

// Where a lane's or a card's pull request stands (ov-317, team mode T6): the
// runner derives the stage on read and sends its kind and facts; this words it
// in the apps' own style. The Android twin of AgentKit's `PlanPrStage.swift`,
// rule for rule: Building is hidden, "PR state unknown" is added to the state
// and never replaces it, only Changes Requested, Checks Failing and Merge
// Conflicts are drawn in color and they lead with the problem, and a reviewer's
// login is cut in the middle. Every field is optional on the wire: a runner
// before the stage sends none, and a kind this build does not know reads as
// [PlanPrStageKind.UNSPECIFIED], which says nothing.

/** The stage's kind, as `plan --json` spells it. */
enum class PlanPrStageKind(val wire: String) {
    BUILDING("building"), AGENT_REVIEW("agent_review"), FIXING("fixing"),
    WAITING_ON_REVIEWER("waiting_on_reviewer"), CHANGES_REQUESTED("changes_requested"),
    APPROVED_CHECKS_RUNNING("approved_checks_running"), APPROVED_CHECKS_FAILING("approved_checks_failing"),
    APPROVED("approved"), APPROVED_CONFLICTS("approved_conflicts"), QUEUED("queued"), MERGED("merged"), CLOSED("closed"),

    /** The runner could not ask GitHub. Never the same as [BUILDING], which is a runner that asked and found no pull request. */
    UNKNOWN("unknown"),

    /** A kind a newer runner sent. */
    UNSPECIFIED("");

    companion object {
        fun parse(wire: String?): PlanPrStageKind = entries.firstOrNull { it.wire == wire && wire != "" } ?: UNSPECIFIED
    }
}

data class PlanPrStage(
    val kind: PlanPrStageKind,
    /** The runner's own words, for a kind this build cannot word. */
    val label: String = "",
    /** Whom a review waits on, for [PlanPrStageKind.WAITING_ON_REVIEWER]: logins and team slugs, in GitHub's order. */
    val reviewers: List<String> = emptyList(),
    /** From 1, for [PlanPrStageKind.QUEUED]. */
    val queuePosition: Int = 0,
    /** 0 for none. */
    val prNumber: Int = 0,
    val prUrl: String = "",
    val unresolvedThreads: Int? = null,
    val checks: String = "unknown",
) {
    /** The stage in the Plan view's words: sentence case for a state, and a reviewer as GitHub spells the login. Null for a kind this build cannot word and the runner gave no words for. */
    val words: String?
        get() = when (kind) {
            PlanPrStageKind.BUILDING -> "Building"
            PlanPrStageKind.AGENT_REVIEW -> "Agent review"
            PlanPrStageKind.FIXING -> "Fixing"
            PlanPrStageKind.WAITING_ON_REVIEWER -> waiting(reviewers)
            PlanPrStageKind.CHANGES_REQUESTED -> "Changes requested"
            PlanPrStageKind.APPROVED_CHECKS_RUNNING -> "Approved, checks running"
            PlanPrStageKind.APPROVED_CHECKS_FAILING -> "Checks failing"
            PlanPrStageKind.APPROVED -> "Approved"
            PlanPrStageKind.APPROVED_CONFLICTS -> "Merge conflicts"
            PlanPrStageKind.QUEUED -> if (queuePosition > 0) "Queued, position $queuePosition" else "Queued"
            PlanPrStageKind.MERGED -> "Merged"
            PlanPrStageKind.CLOSED -> "Closed"
            PlanPrStageKind.UNKNOWN -> "PR state unknown"
            PlanPrStageKind.UNSPECIFIED -> label.ifEmpty { null }
        }

    /** What stays in the gray state line when [words] leads with the problem: it is approved, and the checks or the merge need a person. */
    val stateWords: String?
        get() = if (kind == PlanPrStageKind.APPROVED_CHECKS_FAILING || kind == PlanPrStageKind.APPROVED_CONFLICTS) "Approved" else null

    /** The stage alone in one phrase, state first, for a line with no room for two ("Approved, Checks Failing"). */
    val fullWords: String?
        get() = words?.let { w -> stateWords?.let { "$it, ${w.replaceFirstChar { c -> c.lowercase() }}" } ?: w }

    /** Changes requested, checks failing, conflicts: the stages that need a person, and so the only ones drawn in color. */
    val needsAttention: Boolean
        get() = kind == PlanPrStageKind.CHANGES_REQUESTED || kind == PlanPrStageKind.APPROVED_CHECKS_FAILING ||
            kind == PlanPrStageKind.APPROVED_CONFLICTS

    /** What a row draws for the stage: the problem alone when a person has to act, else one phrase. */
    val rowWords: String? get() = if (needsAttention) words else fullWords ?: words

    /** What a row says, or null when the lane's own state word already says it ("Building" under a building lane) or this build cannot word it. */
    val shown: String? get() = if (kind == PlanPrStageKind.BUILDING) null else words

    /** Whether the pull request is a draft: the runner derives Agent Review and Fixing from a draft and from nothing else. */
    val isDraft: Boolean get() = kind == PlanPrStageKind.AGENT_REVIEW || kind == PlanPrStageKind.FIXING

    /** Whether a person may mark it ready for review from here: a draft the runner numbered. */
    val canMarkReady: Boolean get() = isDraft && prNumber > 0

    /** "PR 31 · 2 threads open": the facts under the words, null without a pull request. */
    val detail: String?
        get() {
            if (prNumber <= 0) return null
            val parts = mutableListOf("PR $prNumber")
            unresolvedThreads?.takeIf { it > 0 }?.let { parts += if (it == 1) "1 thread open" else "$it threads open" }
            return parts.joinToString(" · ")
        }

    /** The pull request's page, only when it is a github.com link (see [GitHubLink]). */
    val link: String? get() = GitHubLink.url(prUrl)

    companion object {
        /** "Waiting on alice", "Waiting on alice and bob", "Waiting on alice and 2 others", and "Waiting on a reviewer" when no one was asked by name. */
        fun waiting(reviewers: List<String>): String = when (reviewers.size) {
            0 -> "Waiting on a reviewer"
            1 -> "Waiting on ${reviewers[0]}"
            2 -> "Waiting on ${reviewers[0]} and ${reviewers[1]}"
            else -> "Waiting on ${reviewers[0]} and ${reviewers.size - 1} others"
        }

        /** A stage off the plan's JSON, or null when the runner sent none. */
        fun decode(element: JsonElement?): PlanPrStage? {
            val o = element as? JsonObject ?: return null
            fun text(name: String) = (o[name] as? JsonElement)?.takeIf { it !is JsonNull }?.jsonPrimitive?.contentOrNull
            return PlanPrStage(
                kind = PlanPrStageKind.parse(text("kind")),
                label = text("label") ?: "",
                reviewers = (o["reviewers"] as? JsonArray)?.mapNotNull { (it as? JsonElement)?.takeIf { e -> e !is JsonNull }?.jsonPrimitive?.contentOrNull } ?: emptyList(),
                queuePosition = o["queue_position"]?.takeIf { it !is JsonNull }?.jsonPrimitive?.intOrNull ?: 0,
                prNumber = o["pr_number"]?.takeIf { it !is JsonNull }?.jsonPrimitive?.intOrNull ?: 0,
                prUrl = text("pr_url") ?: "",
                unresolvedThreads = o["unresolved_threads"]?.takeIf { it !is JsonNull }?.jsonPrimitive?.intOrNull,
                checks = text("checks") ?: "unknown",
            )
        }
    }
}

/**
 * A link the app may hand to the browser for a pull request or a review thread (ov-315): https, a bare host (no user
 * info, no port, no whitespace or backslash), and that host, whole, is `github.com`. The runner checks the same rule
 * where it reads the note, and the CLI where it is written; this is the third, at the tap, because the link is text
 * from a repository. An Enterprise host is not known to an app, so a thread on one has no button. AgentKit's `GitHubLink`.
 */
object GitHubLink {
    private const val HOST = "github.com"

    /** [link] as the same string when the app may open it, else null. */
    fun url(link: String?): String? {
        if (link.isNullOrEmpty() || link.any { it.isWhitespace() || it == '\\' }) return null
        if (!link.lowercase().startsWith("https://")) return null
        val authority = link.substringAfter("://").substringBefore('/').substringBefore('?').substringBefore('#')
        return link.takeIf { authority.lowercase() == HOST }
    }
}

/** A lane's stage, while it is live and the stage has something to add to the state word. */
val PlanLane.shownStage: PlanPrStage?
    get() = stage?.takeIf { state.isLive && it.shown != null }
