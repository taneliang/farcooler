package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

// Every agent working for a board, as the plan carries them (ov-503, design 6; ov-872): each with its role, the tickets
// it works now and what it is doing now, observed on the runner from its turns, never typed. The Android twin of
// AgentKit's `PlanAgents.swift`.

/** What an agent is. */
enum class AgentRole(val wire: String, val word: String) {
    ORCHESTRATOR("orchestrator", "Orchestrator"),
    BUILDER("builder", "Builder"),
    REVIEWER("reviewer", "Reviewer"),
    LANDER("lander", "Lander"),
    AGENT("agent", "Agent");

    companion object {
        fun parse(word: String?): AgentRole = entries.firstOrNull { it.wire == word } ?: AGENT
    }
}

/** A ticket an agent works now, and what it does for it. */
data class PlanAgentTicket(val task: String, val key: String, val role: WorkRole, val startedAt: Long)

data class PlanAgent(
    val id: String,
    val role: AgentRole,
    val harness: String,
    /** The model that spent the most; empty when none was named. */
    val model: String,
    /** What the orchestrator called it; for a subagent, the description its parent gave it. Empty when nothing kept it. */
    val title: String,
    /** The pane, to open it; null for a subagent, which has none. */
    val terminal: String?,
    /** A subagent's id in its harness; empty for a pane. */
    val subagentId: String,
    /** The tickets it works now, in the order it was given them. */
    val tickets: List<PlanAgentTicket>,
    /** Mid-turn. */
    val working: Boolean,
    /** What it is doing now, in a line; empty when the runner can't see it. */
    val doingNow: String,
    /** Asked to stop and not yet ended: it reads "Stopping". */
    val stopAskedAt: Long?,
    val startedAt: Long,
    val lastAt: Long,
    val spend: PlanSpend,
) {

    companion object {
        fun decodeAll(plan: JsonObject): List<PlanAgent> = (plan["agents"] as? JsonArray).orEmpty().map { e ->
            val o = e.jsonObject
            PlanAgent(
                id = o.text("id"), role = AgentRole.parse(o.maybeText("role")), harness = o.text("harness"),
                model = o.text("model"), title = o.text("title"), terminal = o.maybeText("terminal"),
                subagentId = o.text("subagent_id"),
                tickets = (o["tickets"] as? JsonArray).orEmpty().map { t ->
                    val x = t.jsonObject
                    PlanAgentTicket(x.text("task"), x.text("key"), WorkRole.parse(x.maybeText("role")), x["started_at"]?.jsonPrimitive?.longOrNull ?: 0L)
                },
                working = o["working"]?.jsonPrimitive?.booleanOrNull ?: false,
                doingNow = o.text("doing_now"),
                stopAskedAt = o["stop_asked_at"]?.takeIf { it !is JsonNull }?.jsonPrimitive?.longOrNull,
                startedAt = o["started_at"]?.jsonPrimitive?.longOrNull ?: 0L,
                lastAt = o["last_at"]?.jsonPrimitive?.longOrNull ?: 0L,
                spend = Plan.spend(o["spend"] as? JsonObject),
            )
        }
    }
}

private fun JsonArray?.orEmpty(): List<JsonElement> = this?.toList() ?: emptyList()
private fun JsonObject.text(name: String): String = maybeText(name) ?: ""
private fun JsonObject.maybeText(name: String): String? =
    (this[name] as? JsonElement)?.takeIf { it !is JsonNull }?.jsonPrimitive?.contentOrNull
