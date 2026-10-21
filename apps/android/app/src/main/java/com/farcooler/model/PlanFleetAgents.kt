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
import java.util.Locale

// Every agent working for a board, as the plan carries them (ov-503, design 6): derived on the runner from the turns it
// read, never typed, so the orchestrator and a subagent nobody recorded are in it. Behind `board_agents`. The Android
// twin of AgentKit's `PlanFleetAgents.swift`; `PlanFleetAgent` because `PlanAgent` is a lane's typed agent record.

/** What an agent does. */
enum class PlanFleetRole(val wire: String, val word: String) {
    ORCHESTRATOR("orchestrator", "Orchestrator"),
    BUILD("build", "Builder"),
    REVIEW("review", "Reviewer"),
    FIX("fix", "Fixer"),
    INTEGRATOR("integrator", "Integrator"),
    SUBAGENT("subagent", "Subagent"),
    AGENT("agent", "Agent");

    companion object {
        fun parse(word: String?): PlanFleetRole = entries.firstOrNull { it.wire == word } ?: AGENT
    }
}

data class PlanFleetAgent(
    /** `agent:<agentId>`, `pane:<terminal id>` or `turns:<harness>`. */
    val id: String,
    val role: PlanFleetRole,
    val harness: String,
    /** The model that spent the most; empty when none was named. */
    val model: String,
    /** What the orchestrator called a subagent; empty when nothing kept it. */
    val label: String,
    val laneId: String?,
    val laneName: String,
    val trainId: String?,
    val trainTitle: String,
    val card: PlanCardRef?,
    /** The pane, to open it. */
    val terminal: String?,
    val working: Boolean,
    val startedAt: Long,
    val lastAt: Long,
    val spend: PlanSpend,
) {
    /** "Builder · Sonnet": its role and its model, else its harness. */
    val who: String
        get() {
            val m = PlanWords.model(model)
            return "${role.word} · ${m.ifEmpty { harness }}"
        }

    /** What it works for: the plan, a lane, a train or a card, else "Not on a lane". */
    val worksFor: String
        get() = when {
            role == PlanFleetRole.ORCHESTRATOR -> "Plan"
            laneId != null -> laneName
            trainId != null -> trainTitle
            card != null -> card.key
            else -> PlanWords.NOT_ON_A_LANE
        }

    /** "Working · 22 min" while it works; "Idle · active 1 h ago" for a pane that still runs and is between turns. */
    fun state(now: Long): String =
        when {
            working -> "Working · ${PlanWords.span(now - startedAt)}"
            // An orchestrator with no turn in the window has no last turn to name.
            lastAt > 0 -> "Idle · active ${PlanWords.ago(lastAt, now)}"
            else -> "Idle"
        }

    /** "380K", tokens first; null when no turn stated any. */
    fun tokens(locale: Locale = Locale.getDefault()): String? =
        if (spend.totalTokens > 0) TaskUsageFormat.tokens(spend.totalTokens, locale) else null

    /** "≈$1.10", API-equivalent dollars second; null when no price is known. */
    fun cost(locale: Locale = Locale.getDefault()): String? =
        spend.costMicros?.let { "≈" + TaskUsageFormat.dollars(it, locale) }

    companion object {
        fun decodeAll(plan: JsonObject): List<PlanFleetAgent> = (plan["agents"] as? JsonArray).orEmpty().map { e ->
            val o = e.jsonObject
            val lane = o["lane"] as? JsonObject
            val train = o["train"] as? JsonObject
            PlanFleetAgent(
                id = o.text("id"), role = PlanFleetRole.parse(o.maybeText("role")), harness = o.text("harness"),
                model = o.text("model"), label = o.text("label"),
                laneId = lane?.text("id"), laneName = lane?.text("name") ?: "",
                trainId = train?.text("id"), trainTitle = train?.text("title") ?: "",
                card = (o["card"] as? JsonObject)?.let { PlanCardRef(it.text("task"), it.text("key")) },
                terminal = o.maybeText("terminal"),
                working = o["working"]?.jsonPrimitive?.booleanOrNull ?: false,
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

/**
 * The agents list as the Plan draws it: the agents that exist, working first, by spend. The header's total is the
 * day's, the agents that finished included, as the runner counts it; [spend] is null from a runner before ov-518.
 */
data class PlanFleet(val agents: List<PlanFleetAgent>, val spend: PlanSpend? = null) {
    val isEmpty: Boolean get() = agents.isEmpty()

    val workingCount: Int get() = agents.count { it.working }

    /**
     * Whether the orchestrator is among them. This list is every agent that still exists, the orchestrator and the
     * idle ones included; the Agents card counts the agents the orchestrator has running ([RunningAgents.count]),
     * without it. The two numbers can differ by the orchestrator and by idle agents, so the header says what it counts
     * (ov-660).
     */
    val includesOrchestrator: Boolean get() = agents.any { it.role == PlanFleetRole.ORCHESTRATOR }

    /** What the count says it counts: "5 agents, including the orchestrator". */
    val countWords: String
        get() {
            val noun = if (agents.size == 1) "1 agent" else "${agents.size} agents"
            return if (includesOrchestrator) "$noun, including the orchestrator" else noun
        }

    /** The day's tokens. */
    val totalTokens: Long get() = spend?.totalTokens ?: agents.sumOf { it.spend.totalTokens }

    /** Dollars, only when every agent is priced: a sum with a hole in it would read low. */
    val totalCostMicros: Long?
        get() {
            if (spend != null) return spend.costMicros
            var sum = 0L
            for (agent in agents) sum += agent.spend.costMicros ?: return null
            return sum
        }

 /**
     * "5 agents, including the orchestrator · 4 working · 3.9M tokens in the last 24 hours · ≈$26"; with none alive,
     * "No agents running · 1.2M tokens in the last 24 hours · ≈$12" while the day spent something, and "No agents
     * running." when it spent nothing.
     */
    fun header(locale: Locale = Locale.getDefault()): String {
        if (isEmpty && totalTokens <= 0) return PlanWords.NO_AGENTS
        val parts = if (isEmpty) mutableListOf("No agents running") else mutableListOf(countWords, "$workingCount working")
        parts += "${TaskUsageFormat.tokens(totalTokens, locale)} tokens in the last 24 hours"
        totalCostMicros?.let { parts += "≈" + TaskUsageFormat.dollars(it, locale) }
        return parts.joinToString(" · ")
    }

    companion object {
        /** The list sorted as the Plan draws it: working first, then by tokens, most first. */
        fun of(agents: List<PlanFleetAgent>, spend: PlanSpend? = null): PlanFleet =
            PlanFleet(agents.sortedWith(compareByDescending<PlanFleetAgent> { it.working }.thenByDescending { it.spend.totalTokens }), spend)
    }
}
