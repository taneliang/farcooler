package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.longOrNull

/**
 * One row of a terminal-mode agent's session, as the runner's projector folds
 * it (`crates/core/src/session_log/projector/rows.rs`, ov-363) and
 * `agent.rows` serves it (ov-366): AgentKit's `AgentRow`, read from the same
 * JSON (`test/fixtures/agent-rows.json`).
 *
 * The id never changes once handed out, so a client applies a change to the
 * row it already drew rather than re-diffing the list. [ord] is the row's index
 * at insertion and [rev] the projection's revision when it last changed; both
 * only grow.
 *
 * Read from the shape serde gives the Rust types: snake_case fields, and every
 * enum externally tagged (`{"Tool": {...}}`, `"Running"`,
 * `{"Failed": {"detail": "..."}}`). A kind or a state this build doesn't know
 * becomes [Kind.Unknown] rather than failing the page, so a runner newer than
 * the app still draws every row it can.
 */
data class AgentRow(
    val id: String,
    val ord: Long,
    val rev: Long,
    /** The `Turn` row this one belongs to. */
    val turn: String? = null,
    /** Announced by a hook and not yet confirmed by a transcript record. */
    val provisional: Boolean = false,
    val kind: Kind,
) {
    sealed interface Kind {
        data class OfTurn(val turn: Turn) : Kind
        data class OfProse(val prose: Prose) : Kind
        data class OfThinking(val thinking: Thinking) : Kind
        data class OfTool(val tool: Tool) : Kind
        data class OfSubagent(val subagent: Subagent) : Kind
        data class OfAsk(val ask: Ask) : Kind
        data class OfQueued(val queued: Queued) : Kind
        data class OfNotice(val notice: Notice) : Kind
        data class OfHandoff(val handoff: Handoff) : Kind
        data class OfGap(val gap: Gap) : Kind

        /** A kind this build doesn't know, by its name. */
        data class Unknown(val name: String) : Kind
    }

    /** One prompt and everything the agent did about it. */
    data class Turn(
        val prompt: String,
        /** `Typed`, `Queued`, `Notification`, `Sdk`, `System` or `Other`. */
        val origin: String,
        val startedMs: Long? = null,
        val endedMs: Long? = null,
        val durationMs: Long? = null,
        /** Null while the turn is open. */
        val outcome: Outcome? = null,
        val backgroundRunning: Int = 0,
        /** `Busy`, `Idle` or `Shell`, on the newest turn only. */
        val activity: String? = null,
    ) {
        sealed interface Outcome {
            data object Finished : Outcome
            data object Interrupted : Outcome
            data object Unrecorded : Outcome
            data class Failed(val detail: String) : Outcome
            data class Other(val name: String) : Outcome
        }
    }

    data class Prose(
        val text: String,
        /** The turn's closing answer rather than narration on the way there. */
        val conclusion: Boolean = false,
        val atMs: Long? = null,
    )

    data class Thinking(val startedMs: Long? = null, val endedMs: Long? = null)

    data class Tool(
        val name: String,
        val summary: String,
        val status: Status,
        val startedMs: Long? = null,
        val endedMs: Long? = null,
        val diff: List<Hunk> = emptyList(),
        val filePath: String? = null,
    )

    /** A tool's or a subagent's state, folded to what a row draws. */
    sealed interface Status {
        data object Running : Status
        data object Done : Status
        data object Failed : Status

        /**
         * A subagent's own ending other than done or failed: `Killed`,
         * `Stopped`, or a word this build doesn't know.
         */
        data class Ended(val word: String) : Status
    }

    data class Hunk(
        val oldStart: Int,
        val oldLines: Int,
        val newStart: Int,
        val newLines: Int,
        /** Unified-diff lines, each starting with ` `, `-` or `+`. */
        val lines: List<String>,
    )

    data class Subagent(
        val toolUseId: String,
        val agentType: String,
        val description: String,
        val background: Boolean,
        val status: Status,
        val startedMs: Long? = null,
        val endedMs: Long? = null,
        val toolCount: Int = 0,
        /** Its latest tool call, as `Name summary`. */
        val currentAction: String = "",
        val lastMs: Long? = null,
    )

    data class Ask(
        /** `Question`, `Permission` or `PlanExit`. */
        val kind: String,
        val text: String,
        val tool: String? = null,
        val askedMs: Long? = null,
        val answered: Boolean = false,
    )

    data class Queued(
        val text: String,
        /** `Waiting`, `Sent` or `Withdrawn`. */
        val state: String,
        val atMs: Long? = null,
    )

    data class Notice(
        /** `Compacted`, `Cleared`, `Resumed`, `Command` or `ApiError`. */
        val kind: String,
        val text: String,
        val atMs: Long? = null,
    )

    data class Handoff(val reason: String, val atMs: Long? = null)

    data class Gap(
        /** `Unparsed`, `TooLarge`, `Rewritten`, or `Unknown <name>`. */
        val reason: String,
        val count: Int,
    )
}

/** A page of rows: `agent.rows {terminal, before?, limit?}`'s answer as the client core spells it. */
data class AgentRowPage(
    val epoch: Long,
    val rev: Long,
    val moreBefore: Boolean,
    /** Oldest first. */
    val rows: List<AgentRow>,
) {
    companion object {
        fun decode(body: JsonObject): AgentRowPage = AgentRowPage(
            epoch = AgentRowJson.long(body["epoch"]),
            rev = AgentRowJson.long(body["rev"]),
            moreBefore = (body["moreBefore"] as? JsonPrimitive)?.booleanOrNull ?: false,
            rows = (body["rows"] as? JsonArray).orEmpty().mapNotNull { (it as? JsonObject)?.let(AgentRowJson::row) },
        )
    }
}

/** What changed after a revision: `agent.rows_follow`'s answer. */
data class AgentRowChanges(
    val epoch: Long,
    val rev: Long,
    /** The runner can't say what changed (a new epoch, or too much at once): page again. */
    val reset: Boolean,
    val changes: List<Change>,
) {
    sealed interface Change {
        data class Insert(val row: AgentRow) : Change
        data class Update(val row: AgentRow) : Change
        data class Remove(val id: String, val rev: Long) : Change
    }

    companion object {
        fun decode(body: JsonObject): AgentRowChanges {
            val changes = (body["changes"] as? JsonArray).orEmpty().mapNotNull { item ->
                val change = item as? JsonObject ?: return@mapNotNull null
                val id = (change["id"] as? JsonPrimitive)?.contentOrNull ?: return@mapNotNull null
                val row = (change["row"] as? JsonObject)?.let(AgentRowJson::row)
                when ((change["kind"] as? JsonPrimitive)?.contentOrNull) {
                    "remove" -> Change.Remove(id, AgentRowJson.long(change["rev"]))
                    "insert" -> row?.let(Change::Insert)
                    else -> row?.let(Change::Update)
                }
            }
            return AgentRowChanges(
                epoch = AgentRowJson.long(body["epoch"]),
                rev = AgentRowJson.long(body["rev"]),
                reset = (body["reset"] as? JsonPrimitive)?.booleanOrNull ?: false,
                changes = changes,
            )
        }
    }
}

/**
 * The wire, read by hand: every enum on it is externally tagged, and a tolerant
 * reader is shorter than ten serializers and never throws on a field a newer
 * runner added or dropped.
 */
internal object AgentRowJson {
    fun long(value: JsonElement?): Long = (value as? JsonPrimitive)?.longOrNull ?: 0L
    private fun int(value: JsonElement?): Int = long(value).toInt()
    private fun ms(value: JsonElement?): Long? = (value as? JsonPrimitive)?.longOrNull
    private fun string(value: JsonElement?): String? = (value as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull
    private fun bool(value: JsonElement?): Boolean? = (value as? JsonPrimitive)?.booleanOrNull

    /** An externally tagged enum: `"Name"` or `{"Name": payload}`. */
    private fun tag(value: JsonElement?): Pair<String, JsonElement?>? = when (value) {
        is JsonPrimitive -> if (value.isString) value.content to null else null
        is JsonObject -> value.entries.firstOrNull()?.let { it.key to it.value }
        else -> null
    }

    /** One serialized `projector::Row`, or null when it has no id. */
    fun row(json: JsonObject): AgentRow? {
        val id = string(json["id"]) ?: return null
        return AgentRow(
            id = id,
            ord = long(json["ord"]),
            rev = long(json["rev"]),
            turn = string(json["turn"]),
            provisional = bool(json["provisional"]) ?: false,
            kind = kind(json["kind"]),
        )
    }

    private fun kind(value: JsonElement?): AgentRow.Kind {
        val (name, payload) = tag(value) ?: return AgentRow.Kind.Unknown("")
        val p = payload as? JsonObject ?: JsonObject(emptyMap())
        fun text(key: String) = string(p[key]).orEmpty()
        return when (name) {
            "Turn" -> AgentRow.Kind.OfTurn(
                AgentRow.Turn(
                    prompt = text("prompt"),
                    origin = tag(p["origin"])?.first ?: "Other",
                    startedMs = ms(p["started_ms"]),
                    endedMs = ms(p["ended_ms"]),
                    durationMs = ms(p["duration_ms"]),
                    outcome = tag(p["outcome"])?.let { (outcome, detail) ->
                        when (outcome) {
                            "Finished" -> AgentRow.Turn.Outcome.Finished
                            "Interrupted" -> AgentRow.Turn.Outcome.Interrupted
                            "Unrecorded" -> AgentRow.Turn.Outcome.Unrecorded
                            "Failed" -> AgentRow.Turn.Outcome.Failed(string((detail as? JsonObject)?.get("detail")).orEmpty())
                            else -> AgentRow.Turn.Outcome.Other(outcome)
                        }
                    },
                    backgroundRunning = int(p["background_running"]),
                    activity = tag(p["activity"])?.first,
                ),
            )
            "Prose" -> AgentRow.Kind.OfProse(
                AgentRow.Prose(text("text"), bool(p["conclusion"]) ?: false, ms(p["at_ms"])),
            )
            "Thinking" -> AgentRow.Kind.OfThinking(AgentRow.Thinking(ms(p["started_ms"]), ms(p["ended_ms"])))
            "Tool" -> AgentRow.Kind.OfTool(
                AgentRow.Tool(
                    name = text("name"),
                    summary = text("summary"),
                    status = status(p["status"]),
                    startedMs = ms(p["started_ms"]),
                    endedMs = ms(p["ended_ms"]),
                    diff = (p["diff"] as? JsonArray).orEmpty().mapNotNull { it as? JsonObject }.map { h ->
                        AgentRow.Hunk(
                            oldStart = int(h["old_start"]),
                            oldLines = int(h["old_lines"]),
                            newStart = int(h["new_start"]),
                            newLines = int(h["new_lines"]),
                            lines = (h["lines"] as? JsonArray).orEmpty().mapNotNull(::string),
                        )
                    },
                    filePath = string(p["file_path"]),
                ),
            )
            "Subagent" -> AgentRow.Kind.OfSubagent(
                AgentRow.Subagent(
                    toolUseId = text("tool_use_id"),
                    agentType = text("agent_type"),
                    description = text("description"),
                    background = bool(p["background"]) ?: false,
                    status = status(p["status"]),
                    startedMs = ms(p["started_ms"]),
                    endedMs = ms(p["ended_ms"]),
                    toolCount = int(p["tool_count"]),
                    currentAction = text("current_action"),
                    lastMs = ms(p["last_ms"]),
                ),
            )
            "Ask" -> AgentRow.Kind.OfAsk(
                AgentRow.Ask(
                    kind = tag(p["kind"])?.first.orEmpty(),
                    text = text("text"),
                    tool = string(p["tool"]),
                    askedMs = ms(p["asked_ms"]),
                    answered = bool(p["answered"]) ?: false,
                ),
            )
            "Queued" -> AgentRow.Kind.OfQueued(
                AgentRow.Queued(text("text"), tag(p["state"])?.first ?: "Waiting", ms(p["at_ms"])),
            )
            "Notice" -> AgentRow.Kind.OfNotice(
                AgentRow.Notice(tag(p["kind"])?.first.orEmpty(), text("text"), ms(p["at_ms"])),
            )
            "Handoff" -> AgentRow.Kind.OfHandoff(AgentRow.Handoff(text("reason"), ms(p["at_ms"])))
            "Gap" -> AgentRow.Kind.OfGap(
                AgentRow.Gap(
                    reason = tag(p["reason"])?.let { (reason, extra) ->
                        string(extra)?.let { "$reason $it" } ?: reason
                    }.orEmpty(),
                    count = int(p["count"]),
                ),
            )
            else -> AgentRow.Kind.Unknown(name)
        }
    }

    private fun status(value: JsonElement?): AgentRow.Status = when (val name = tag(value)?.first) {
        "Running" -> AgentRow.Status.Running
        "Done", "Completed" -> AgentRow.Status.Done
        "Failed" -> AgentRow.Status.Failed
        null -> AgentRow.Status.Ended("")
        else -> AgentRow.Status.Ended(name)
    }
}
