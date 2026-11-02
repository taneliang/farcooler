package com.farcooler.model

import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.longOrNull

/** Projector-owned literal intent only; never a receipt or navigation authority. */
data class LiteralOperation(
    val kind: String,
    val target: String,
    val preview: String?,
    val previewClipped: Boolean,
    val lifecycle: String,
    val inputClipped: Boolean,
    val outputAvailable: Boolean,
    val outputClipped: Boolean,
) {
    val words: String get() = when (lifecycle) {
        "running" -> "Command running"
        "completed" -> "Command completed"
        "failed" -> "Command failed"
        "declined" -> "Command declined"
        else -> "Outcome unconfirmed"
    }
    val verb: String get() = when {
        kind == "task_show" -> "Read task"
        kind == "message_orchestrator" -> "Message orchestrator"
        kind == "task_note_decision" -> "Add decision"
        kind.startsWith("task_note_") -> "Add ${kind.removePrefix("task_note_")} note"
        else -> "Request status ${kind.removePrefix("task_set_").replace('_', ' ')}"
    }
    val summary: String get() = if (kind == "message_orchestrator" ) verb else "$verb · $target"
    val status: AgentRow.Status get() = when (lifecycle) {
        "running" -> AgentRow.Status.Running
        "completed" -> AgentRow.Status.Done
        "failed", "declined" -> AgentRow.Status.Failed
        else -> AgentRow.Status.Ended("Outcome unconfirmed")
    }
    val containsControls: Boolean get() = preview?.any { (it.isISOControl() && it != '\n' && it != '\t') || it in '\u202A'..'\u202E' || it in '\u2066'..'\u2069' } ?: false

    companion object {
        private val kinds = setOf("task_show", "message_orchestrator") +
            listOf("decision", "finding", "question", "answer", "progress", "comment").map { "task_note_$it" } +
            listOf("backlog", "todo", "needs_decision", "in_progress", "in_review", "done", "cancelled").map { "task_set_$it" }
        /** Wire and retained in-memory rows share this fail-closed optional decoder. */
        fun decode(value: JsonElement?): LiteralOperation? {
            val p = value as? JsonObject ?: return null
            if (p.toString().toByteArray(Charsets.UTF_8).size > 1024) return null
            fun text(key: String): String? = (p[key] as? JsonPrimitive)?.takeIf { it.isString }?.content
            fun bool(key: String): Boolean? = (p[key] as? JsonPrimitive)?.takeUnless { it.isString }?.booleanOrNull
            if ((p["version"] as? JsonPrimitive)?.takeUnless { it.isString }?.longOrNull != 1L || text("provenance") != "literal_intent") return null
            val kind = text("kind")?.takeIf { it in kinds } ?: return null
            val target = text("target") ?: return null
            if (target.toByteArray(Charsets.UTF_8).size > 64 || if (kind == "message_orchestrator") target != "orchestrator" else !target.matches(Regex("[A-Za-z0-9]+-[0-9]+"))) return null
            val preview = if (p.containsKey("preview")) text("preview") ?: return null else null
            if (preview != null && (preview.codePointCount(0, preview.length) > 280 || preview.count { it == '\n' } > 3)) return null
            val lifecycle = text("lifecycle")?.takeIf { it in setOf("running", "completed", "failed", "declined", "unconfirmed") } ?: return null
            return LiteralOperation(kind, target, preview, bool("preview_clipped") ?: return null, lifecycle,
                bool("input_clipped") ?: return null, bool("output_available") ?: return null, bool("output_clipped") ?: return null)
        }
    }
}
