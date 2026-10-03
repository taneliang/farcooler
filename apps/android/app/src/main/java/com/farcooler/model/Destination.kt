package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.intOrNull

/**
 * Where the app is, or should go (ov-182, ov-183). AgentKit's `Destination`,
 * mirrored case for case.
 *
 * One value for two jobs. **Restore**: "where I was", in the ids this device
 * already holds ([Runner.host], a task's id, a terminal id). **Notification**:
 * "what this is about", in portable ids ([Runner.id], a task's key and
 * repository, a terminal id). [DestinationResolver] turns either into a place
 * that exists.
 *
 * The wire form is JSON, `{"v":1, …}`, compact with sorted keys: the same
 * bytes AgentKit writes, and `test/fixtures/destinations.json` holds both to
 * them.
 */
data class Destination(
    /** The runner, as much as is known of it. Empty for Needs You. */
    val runner: Runner = Runner(),
    val place: Place,
    /** The task tab, kept only while [place] is the task and it offers it. */
    val tab: Tab? = null,
    /** The workspace screen's segment, kept only while [place] is a workspace. */
    val segment: Segment? = null,
    /** The terminal the keyboard was in, or the pane a notice is about. */
    val pane: String? = null,
    /** The agent pane a task's Agent tab shows. */
    val agent: String? = null,
    /** A decision or a question waiting on the task: open with it in front. */
    val question: Boolean = false,
) {
    /** A runner, by either or both of its names. */
    data class Runner(
        /** This device's own handle for it (`Host.id` here). */
        val host: String? = null,
        /** Its `Host.runner_id`, the portable name a push carries. Compared without case. */
        val id: String? = null,
    ) {
        val isEmpty: Boolean get() = host == null && id == null
    }

    /** A task, by id when read here, by key (and repository) when a notice names it. */
    data class TaskRef(val id: String? = null, val key: String? = null, val repository: String? = null)

    /** The level the destination is at. */
    sealed interface Place {
        /** The workspace this place is in, when it says. */
        val workspace: String?

        data object NeedsYou : Place {
            override val workspace: String? get() = null
        }

        /** A workspace's board, as it opens by default. */
        data class Workspace(val id: String) : Place {
            override val workspace: String get() = id
        }

        data class Orchestrator(override val workspace: String) : Place
        /** A finished status's History page (`done`, `cancelled`). */
        data class History(override val workspace: String, val status: String) : Place
        /** A task. The workspace is null when a notice names only a key. */
        data class Task(override val workspace: String?, val task: TaskRef) : Place
        /** A worktree. A null workspace is a loose one, or one a notice didn't say. */
        data class Worktree(val id: String, override val workspace: String?) : Place

        /** A pane, by terminal id, before it's resolved to its worktree. */
        data class Terminal(val id: String) : Place {
            override val workspace: String? get() = null
        }

        /**
         * The places to fall back to when this one is gone, nearest first, as
         * far as the place itself knows: its workspace, for those that say it.
         */
        val ancestors: List<Place>
            get() = when (this) {
                NeedsYou, is Workspace, is Terminal -> emptyList()
                is Orchestrator -> listOf(Workspace(workspace))
                is History -> listOf(Workspace(workspace))
                is Task -> listOfNotNull(workspace?.let(::Workspace))
                is Worktree -> listOfNotNull(workspace?.let(::Workspace))
            }
    }

    enum class Tab(val wire: String) {
        OVERVIEW("overview"), AGENT("agent"), CHANGES("changes");

        companion object {
            fun of(wire: String?): Tab? = entries.firstOrNull { it.wire == wire }
        }
    }

    enum class Segment(val wire: String) {
        ORCHESTRATOR("orchestrator"), BOARD("board"), WORKTREES("worktrees");

        companion object {
            fun of(wire: String?): Segment? = entries.firstOrNull { it.wire == wire }
        }
    }

    /** The JSON object, keys sorted. */
    fun toJson(): JsonObject {
        val out = sortedMapOf<String, JsonElement>("v" to JsonPrimitive(VERSION), "place" to placeJson(place))
        val runnerObject = sortedMapOf<String, JsonElement>()
        runner.host?.let { runnerObject["host"] = JsonPrimitive(it) }
        runner.id?.let { runnerObject["id"] = JsonPrimitive(it) }
        if (runnerObject.isNotEmpty()) out["runner"] = JsonObject(runnerObject)
        tab?.let { out["tab"] = JsonPrimitive(it.wire) }
        segment?.let { out["segment"] = JsonPrimitive(it.wire) }
        pane?.let { out["pane"] = JsonPrimitive(it) }
        agent?.let { out["agent"] = JsonPrimitive(it) }
        if (question) out["question"] = JsonPrimitive(true)
        return JsonObject(out)
    }

    /** The encoding as a string: compact and keys sorted, as AgentKit writes it. */
    fun encoded(): String = toJson().toString()

    companion object {
        /** The encoding's version. A value of any other version decodes as null. */
        const val VERSION = 1

        /** Needs You, with nothing else said. */
        val NEEDS_YOU = Destination(place = Place.NeedsYou)

        /**
         * A destination from its encoding, or null: an unknown version, place
         * kind, or a place missing what it needs. An unknown tab or segment is
         * dropped alone, and unknown keys are ignored.
         */
        fun decode(encoded: String): Destination? {
            val element = runCatching { Json.parseToJsonElement(encoded) }.getOrNull() ?: return null
            return fromJson(element as? JsonObject ?: return null)
        }

        fun fromJson(json: JsonObject): Destination? {
            val version = (json["v"] as? JsonPrimitive)?.takeIf { !it.isString }?.intOrNull
            if (version != VERSION) return null
            val place = placeOf(json["place"] as? JsonObject ?: return null) ?: return null
            val runner = json["runner"] as? JsonObject
            return Destination(
                runner = Runner(host = runner?.string("host"), id = runner?.string("id")?.ifEmpty { null }),
                place = place,
                tab = Tab.of(json.string("tab")),
                segment = Segment.of(json.string("segment")),
                pane = json.string("pane")?.ifEmpty { null },
                agent = json.string("agent")?.ifEmpty { null },
                question = (json["question"] as? JsonPrimitive)?.takeIf { !it.isString }?.booleanOrNull ?: false,
            )
        }

        private fun placeOf(json: JsonObject): Place? {
            val workspace = json.string("workspace")?.ifEmpty { null }
            return when (json.string("kind")) {
                "needs-you" -> Place.NeedsYou
                "workspace" -> workspace?.let(Place::Workspace)
                "orchestrator" -> workspace?.let(Place::Orchestrator)
                "history" -> {
                    val status = json.string("status")?.ifEmpty { null }
                    if (workspace == null || status == null) null else Place.History(workspace, status)
                }
                "task" -> {
                    val ref = json["task"] as? JsonObject ?: return null
                    val task = TaskRef(
                        id = ref.string("id")?.ifEmpty { null },
                        key = ref.string("key")?.ifEmpty { null },
                        repository = ref.string("repository")?.ifEmpty { null },
                    )
                    if (task.id == null && task.key == null) null else Place.Task(workspace, task)
                }
                "worktree" -> json.string("worktree")?.ifEmpty { null }?.let { Place.Worktree(it, workspace) }
                "terminal" -> json.string("terminal")?.ifEmpty { null }?.let(Place::Terminal)
                else -> null
            }
        }

        private fun placeJson(place: Place): JsonObject {
            val out = sortedMapOf<String, JsonElement>()
            fun put(key: String, value: String?) { value?.let { out[key] = JsonPrimitive(it) } }
            when (place) {
                Place.NeedsYou -> put("kind", "needs-you")
                is Place.Workspace -> { put("kind", "workspace"); put("workspace", place.id) }
                is Place.Orchestrator -> { put("kind", "orchestrator"); put("workspace", place.workspace) }
                is Place.History -> {
                    put("kind", "history"); put("workspace", place.workspace); put("status", place.status)
                }
                is Place.Task -> {
                    put("kind", "task"); put("workspace", place.workspace)
                    val ref = sortedMapOf<String, JsonElement>()
                    place.task.id?.let { ref["id"] = JsonPrimitive(it) }
                    place.task.key?.let { ref["key"] = JsonPrimitive(it) }
                    place.task.repository?.let { ref["repository"] = JsonPrimitive(it) }
                    out["task"] = JsonObject(ref)
                }
                is Place.Worktree -> { put("kind", "worktree"); put("worktree", place.id); put("workspace", place.workspace) }
                is Place.Terminal -> { put("kind", "terminal"); put("terminal", place.id) }
            }
            return JsonObject(out)
        }

        /** A string member, or null for one that's absent or not a string. */
        private fun JsonObject.string(key: String): String? =
            (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content
    }
}
