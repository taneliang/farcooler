package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

// Where a ticket's or a card's pull request stands (ov-317, team mode T6): the
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
    /** Whether the pull request is a draft: the runner derives Agent Review and Fixing from a draft and from nothing else. */
    val isDraft: Boolean get() = kind == PlanPrStageKind.AGENT_REVIEW || kind == PlanPrStageKind.FIXING

    /** Whether a person may mark it ready for review from here: a draft the runner numbered. */
    val canMarkReady: Boolean get() = isDraft && prNumber > 0

    /** The pull request's page, only when it is a github.com link (see [GitHubLink]). */
    val link: String? get() = GitHubLink.url(prUrl)

    companion object {
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
