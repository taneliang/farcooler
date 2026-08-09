package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

enum class Role(val wire: String) {
    USER("User"),
    AGENT("Agent"),
    THOUGHT("Thought");

    companion object {
        fun parse(raw: String?): Role = entries.firstOrNull { it.wire == raw } ?: AGENT
    }
}

enum class ToolStatus(val wire: String) {
    PENDING("Pending"),
    IN_PROGRESS("InProgress"),
    COMPLETED("Completed"),
    FAILED("Failed");

    companion object {
        fun parse(raw: String?): ToolStatus = entries.firstOrNull { it.wire == raw } ?: PENDING
    }
}

/**
 * Why history is missing, as the daemon named it.
 *
 * Every case but [LoadFailed] is a serde unit variant, which travels as a bare
 * JSON string. [LoadFailed] carries the adapter's own message, so serde encodes
 * it as a struct variant instead — a single-key object wrapping
 * `{"detail": …}` — and [parse] has to try the string shape first and fall back
 * to the keyed shape only for that one case.
 */
sealed interface GapReason {
    data object RingTrimmed : GapReason

    /**
     * The agent declared, at `initialize`, that it does not implement
     * `session/load` at all. `session/load` was never even attempted.
     */
    data object LoadUnsupported : GapReason

    /**
     * Nothing was recorded for this session yet.
     *
     * Not a failure: a claude or codex terminal switched to chat before its
     * first turn has no transcript to load, and that is the common case, not an
     * exotic one.
     */
    data object LoadEmpty : GapReason

    /**
     * `session/load` was attempted and the agent refused it for a reason other
     * than "nothing recorded yet". Carries the adapter's own message.
     */
    data class LoadFailed(val detail: String) : GapReason

    data object Unparsed : GapReason

    companion object {
        fun parse(element: JsonElement?): GapReason {
            if (element == null) return Unparsed
            // The common shape first: every case but LoadFailed is a bare
            // string on the wire.
            val raw = (element as? kotlinx.serialization.json.JsonPrimitive)?.contentOrNull
            if (raw != null) {
                return when (raw) {
                    "RingTrimmed" -> RingTrimmed
                    "LoadUnsupported" -> LoadUnsupported
                    "LoadEmpty" -> LoadEmpty
                    "Unparsed" -> Unparsed
                    // A reason a later daemon invented and this build does not
                    // know about yet — same rule as the top-level event: a gap
                    // this build cannot name precisely is still a gap, never a
                    // silent drop.
                    else -> Unparsed
                }
            }
            val keyed = element as? JsonObject ?: return Unparsed
            val payload = keyed["LoadFailed"]?.jsonObject ?: return Unparsed
            return LoadFailed(payload["detail"]?.jsonPrimitive?.contentOrNull.orEmpty())
        }
    }
}

data class Diff(val path: String, val oldText: String?, val newText: String)

data class PlanEntry(val content: String, val priority: String, val status: String)

/**
 * One selectable option: a mode, or a model.
 *
 * Carries the human [name] as well as the [id]. A picker built from ids alone
 * offers `acceptEdits` and `bypassPermissions` — the wire's words, not
 * anyone's.
 */
data class AgentChoice(val id: String, val name: String, val description: String = "")

/**
 * One thing a user can change about a session.
 *
 * ACP's stabilised generic form. The client renders a control per option rather
 * than knowing in advance that "mode" and "model" exist — which is how a
 * subagent picker and a thought level arrive with no new code.
 */
data class ConfigOption(
    val id: String,
    val name: String,
    val description: String,
    /**
     * `mode`, `model`, `model_config`, `thought_level`, or empty. A hint for
     * ordering only — never a reason to special-case one.
     */
    val category: String,
    /** `select` or `boolean`. */
    val kind: String,
    /** An option id for a select; `"true"`/`"false"` for a boolean. */
    val currentValue: String,
    val options: List<AgentChoice>,
) {
    val isBoolean: Boolean get() = kind == "boolean"
    val isOn: Boolean get() = currentValue == "true"
}

data class PermissionOption(val id: String, val name: String, val kind: String)

/**
 * What a finished subagent reports about itself.
 *
 * Arrives once, on the dispatching call's final update. Everything here is
 * structured on the wire — the same numbers also appear inside the tool's raw
 * output as a `<usage>` text blob, and reading them from there would break the
 * first time the adapter reworded a line nobody promised to keep.
 */
data class SubagentSummary(
    val agentType: String,
    val model: String,
    val tokens: Long,
    val toolUses: Long,
    val durationMs: Long,
    val status: String,
)

/** Only its presence matters here — the bytes are the daemon's business. */
data class QueuedImage(val mime: String)

/**
 * A prompt waiting for the current turn to end.
 *
 * Far Cooler holds these rather than handing them straight to the agent, which
 * is the only reason one can still be rewritten or taken back — a prompt the
 * adapter has is gone.
 */
data class QueuedPrompt(val id: String, val text: String, val images: List<QueuedImage> = emptyList()) {
    /**
     * How many pictures are waiting with it.
     *
     * Decoded because a message can be an image and nothing else, and a queued
     * row that showed only [text] then rendered an empty bubble — which is
     * indistinguishable from the attachment having been dropped on the floor.
     */
    val imageCount: Int get() = images.size
}

sealed interface AgentEvent {
    data class SessionStarted(
        val sessionId: String,
        val agentMode: String?,
        val availableModes: List<AgentChoice>,
        val model: String?,
        val availableModels: List<AgentChoice>,
        val configOptions: List<ConfigOption>,
        val availableCommands: List<AgentChoice>,
        /**
         * Which protocol is carrying this conversation: `acp`, `claude`, or
         * `codex`.
         *
         * Defaults to `acp` rather than being nullable, because that is what a
         * session which never said it is: ACP was the only backend that
         * existed when those transcripts were written.
         */
        val backend: String = "acp",
    ) : AgentEvent

    /**
     * [parent] names the dispatch this belongs to when a subagent produced it.
     * Null is the ordinary case: the agent itself spoke.
     */
    data class Message(val role: Role, val text: String, val parent: String?) : AgentEvent

    /**
     * [subagent] marks a dispatch — a call that OWNS a block rather than being
     * a row inside one.
     */
    data class ToolCall(
        val id: String,
        val title: String,
        val kind: String,
        val status: ToolStatus,
        val locations: List<String>,
        val parent: String?,
        val subagent: Boolean,
    ) : AgentEvent

    data class ToolUpdate(
        val id: String,
        val status: ToolStatus,
        val title: String?,
        val content: String?,
        val diff: Diff?,
        val locations: List<String>,
        val parent: String?,
        val subagent: SubagentSummary?,
    ) : AgentEvent

    data class Plan(val entries: List<PlanEntry>) : AgentEvent

    data class Permission(
        val id: String,
        val toolCall: String,
        val options: List<PermissionOption>,
    ) : AgentEvent

    data class Resolved(val id: String, val chosen: String) : AgentEvent

    data class ModeSet(val agentMode: String) : AgentEvent

    /** A selector changed — by the user, or by the agent itself. */
    data class ConfigSet(val id: String, val value: String) : AgentEvent

    /** Context-window usage, resent as a turn consumes it. */
    data class Usage(val used: Long, val size: Long) : AgentEvent

    /** What this conversation is called, as the agent names it. */
    data class SessionInfo(val title: String) : AgentEvent

    /** The slash-command menu, resent once per turn. Feeds the `/` picker. */
    data class CommandsAvailable(val commands: List<AgentChoice>) : AgentEvent

    data class TurnEnded(val reason: String) : AgentEvent

    /** Everything written but not yet sent, in order. Sent whole on any change. */
    data class PromptQueue(val items: List<QueuedPrompt>) : AgentEvent

    data class Gap(val reason: GapReason) : AgentEvent

    companion object {
        private val json = Json { ignoreUnknownKeys = true; isLenient = true }

        /**
         * Decode one serialised `farcooler_agent::event::AgentEvent`.
         *
         * Serde's externally-tagged representation: a single-key object whose
         * key names the variant. An unrecognised key is a `Gap(Unparsed)`
         * rather than a throw, because a client one release behind its daemon
         * must still render the session — and rather than a silent skip,
         * because a shorter transcript that looks complete is the one failure
         * this design refuses.
         */
        fun decode(payload: String): AgentEvent {
            val root = runCatching { json.parseToJsonElement(payload).jsonObject }.getOrNull()
                ?: return Gap(GapReason.Unparsed)
            val key = root.keys.firstOrNull() ?: return Gap(GapReason.Unparsed)
            val body = root[key] as? JsonObject ?: return Gap(GapReason.Unparsed)

            return runCatching {
                when (key) {
                    "SessionStarted" -> SessionStarted(
                        sessionId = body.string("session_id"),
                        agentMode = body.stringOrNull("agent_mode"),
                        availableModes = body.choices("available_modes"),
                        model = body.stringOrNull("model"),
                        availableModels = body.choices("available_models"),
                        configOptions = body.configOptions("config_options"),
                        availableCommands = body.choices("available_commands"),
                        backend = body.stringOrNull("backend") ?: "acp",
                    )

                    "Message" -> Message(
                        role = Role.parse(body.stringOrNull("role")),
                        text = body.string("text"),
                        parent = body.stringOrNull("parent"),
                    )

                    "ToolCall" -> ToolCall(
                        id = body.string("id"),
                        title = body.string("title"),
                        kind = body.string("kind"),
                        status = ToolStatus.parse(body.stringOrNull("status")),
                        locations = body.strings("locations"),
                        parent = body.stringOrNull("parent"),
                        // Every subagent field is optional, and has to be: the
                        // daemon skips them when empty, so an event from before
                        // subagents were modelled — which is every event already
                        // in SQLite — carries none of them.
                        subagent = body["subagent"]?.jsonPrimitive?.booleanOrNull ?: false,
                    )

                    "ToolUpdate" -> ToolUpdate(
                        id = body.string("id"),
                        status = ToolStatus.parse(body.stringOrNull("status")),
                        title = body.stringOrNull("title"),
                        content = body.stringOrNull("content"),
                        diff = body.diff("diff"),
                        locations = body.strings("locations"),
                        parent = body.stringOrNull("parent"),
                        subagent = body.subagentSummary("subagent"),
                    )

                    "Plan" -> Plan(body.planEntries("entries"))

                    "Permission" -> Permission(
                        id = body.string("id"),
                        toolCall = body.string("tool_call"),
                        options = body.permissionOptions("options"),
                    )

                    "Resolved" -> Resolved(body.string("id"), body.string("chosen"))
                    "CommandsAvailable" -> CommandsAvailable(body.choices("commands"))
                    "SessionInfo" -> SessionInfo(body.string("title"))
                    "Usage" -> Usage(body.long("used"), body.long("size"))
                    "ConfigSet" -> ConfigSet(body.string("id"), body.string("value"))
                    "ModeSet" -> ModeSet(body.string("agent_mode"))
                    "PromptQueue" -> PromptQueue(body.queuedPrompts("items"))
                    "TurnEnded" -> TurnEnded(body.string("reason"))
                    "Gap" -> Gap(GapReason.parse(body["reason"]))
                    else -> Gap(GapReason.Unparsed)
                }
            }.getOrElse { Gap(GapReason.Unparsed) }
        }
    }
}

data class Sequenced(val seq: Long, val event: AgentEvent)

// MARK: - Field readers
//
// Hand-written rather than `@Serializable`, because serde's externally-tagged
// enum has no single Kotlin shape: the variant is the key, the payload is the
// value, and one of the variants (`Gap`) nests a second enum with the same
// trick. Generated decoders would need a custom serialiser per case anyway, and
// this way every field's default is visible at the point where it is read.

private fun JsonObject.stringOrNull(key: String): String? =
    this[key]?.jsonPrimitive?.contentOrNull

private fun JsonObject.string(key: String): String = stringOrNull(key).orEmpty()

private fun JsonObject.long(key: String): Long = this[key]?.jsonPrimitive?.longOrNull ?: 0

private fun JsonObject.strings(key: String): List<String> =
    this[key]?.jsonArray?.mapNotNull { it.jsonPrimitive.contentOrNull } ?: emptyList()

private fun JsonObject.choices(key: String): List<AgentChoice> =
    this[key]?.jsonArray?.map { element ->
        val item = element.jsonObject
        AgentChoice(
            id = item.string("id"),
            name = item.string("name"),
            description = item.string("description"),
        )
    } ?: emptyList()

private fun JsonObject.configOptions(key: String): List<ConfigOption> =
    this[key]?.jsonArray?.map { element ->
        val item = element.jsonObject
        ConfigOption(
            id = item.string("id"),
            name = item.string("name"),
            description = item.string("description"),
            category = item.string("category"),
            kind = item.string("kind"),
            currentValue = item.string("current_value"),
            options = item.choices("options"),
        )
    } ?: emptyList()

private fun JsonObject.permissionOptions(key: String): List<PermissionOption> =
    this[key]?.jsonArray?.map { element ->
        val item = element.jsonObject
        PermissionOption(
            id = item.string("id"),
            name = item.string("name"),
            kind = item.string("kind"),
        )
    } ?: emptyList()

private fun JsonObject.planEntries(key: String): List<PlanEntry> =
    this[key]?.jsonArray?.map { element ->
        val item = element.jsonObject
        PlanEntry(
            content = item.string("content"),
            priority = item.string("priority"),
            status = item.string("status"),
        )
    } ?: emptyList()

private fun JsonObject.queuedPrompts(key: String): List<QueuedPrompt> =
    this[key]?.jsonArray?.map { element ->
        val item = element.jsonObject
        QueuedPrompt(
            id = item.string("id"),
            text = item.string("text"),
            images = item["images"]?.jsonArray?.map {
                QueuedImage(it.jsonObject.string("mime"))
            } ?: emptyList(),
        )
    } ?: emptyList()

private fun JsonObject.diff(key: String): Diff? {
    val item = this[key] as? JsonObject ?: return null
    return Diff(
        path = item.string("path"),
        oldText = item.stringOrNull("old_text"),
        newText = item.string("new_text"),
    )
}

private fun JsonObject.subagentSummary(key: String): SubagentSummary? {
    val item = this[key] as? JsonObject ?: return null
    return SubagentSummary(
        agentType = item.string("agent_type"),
        model = item.string("model"),
        tokens = item.long("tokens"),
        toolUses = item.long("tool_uses"),
        durationMs = item.long("duration_ms"),
        status = item.string("status"),
    )
}
