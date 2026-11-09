package com.farcooler.model

import com.farcooler.core.NativeClient
import com.farcooler.core.coreIsAvailable
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.util.TimeZone

// The Plan's sections and ticket cards as the shared core builds them (ov-1016, ruling R-112): `crates/client/src/plan_view.rs`
// decides which ticket is in which section, the order of the landings and every word a section, a card, a landing, an
// agent row and a theme tile says. These classes only decode its answer; nothing here decides anything. The phone lays
// them out (`ui/PlanSectionRows.kt`), and the glance band, the strip and the sidebar's ticket list read them in place of
// the sections the app used to work out itself.

@Serializable
enum class PlanViewKind {
    @SerialName("needs_you") NEEDS_YOU,
    @SerialName("landing") LANDING,
    @SerialName("in_review") IN_REVIEW,
    @SerialName("building") BUILDING,
    @SerialName("waiting") WAITING,
    @SerialName("on_landed_landing") ON_LANDED_LANDING,
    @SerialName("agents") AGENTS,
    @SerialName("themes") THEMES,
    @SerialName("landed_today") LANDED_TODAY,
}

@Serializable
data class PlanViewSection(
    val kind: PlanViewKind,
    val title: String = "",
    /** Its rows; 0 draws no count. */
    val count: Int = 0,
    /** What it says with nothing in it. */
    val empty: String? = null,
    /** Drawn folded until opened. */
    val folded: Boolean = false,
    /** Said on a long press of the title. */
    val help: String? = null,
)

@Serializable
enum class PlanViewMark {
    @SerialName("rank") RANK,
    @SerialName("waiting") WAITING,
    @SerialName("building") BUILDING,
    @SerialName("fixing") FIXING,
    @SerialName("reviewing") REVIEWING,
    @SerialName("needs_you") NEEDS_YOU,
    @SerialName("landing") LANDING,
    @SerialName("landed") LANDED,
    @SerialName("dropped") DROPPED,
    @SerialName("unknown") UNKNOWN,
}

@Serializable
enum class PlanViewNoteKind {
    @SerialName("stage") STAGE,
    @SerialName("run") RUN,
    @SerialName("warning") WARNING,
    @SerialName("over_budget") OVER_BUDGET,
}

/** A line under a ticket's second line. */
@Serializable
data class PlanViewNote(
    val kind: PlanViewNoteKind,
    val text: String = "",
    /** Drawn in amber. */
    val attention: Boolean = false,
    val help: String? = null,
)

/** The agent working a ticket, or landing it. */
@Serializable
data class PlanViewAgentRef(
    val id: String = "",
    /** "Builder · Sonnet". */
    val who: String = "",
    /** "380K"; null when no turn stated any. */
    val tokens: String? = null,
    /** Who it is and what it has spent, as its own line under a card's second line ("Builder · sonnet · 380K"). */
    val line: String = "",
    val terminal: String? = null,
    /** The model as recorded, which a screen reader says when [who] shortens it; "" when none is known. */
    val model: String = "",
)

@Serializable
data class PlanViewGate(val label: String = "", val text: String = "", val principal: String = "")

/** The engine's run of a ticket: the step it is on, the gate it waits at. */
@Serializable
data class PlanViewRun(
    val run: String = "",
    @SerialName("phase_label") val phaseLabel: String = "",
    val step: String? = null,
    val gate: PlanViewGate? = null,
    val attention: Boolean = false,
)

/** A ticket's row in In review, Building, Waiting or Waiting for the next landing. */
@Serializable
data class PlanViewTicket(
    /** The card's task id. */
    val id: String = "",
    val key: String = "",
    val heading: String = "",
    val theme: String? = null,
    val stage: String = "",
    /** "Fixing · round 1", "Waiting · 2nd". */
    val status: String = "",
    /** The line under the title. */
    val second: String = "",
    val rank: Int? = null,
    val mark: PlanViewMark = PlanViewMark.UNKNOWN,
    @SerialName("mark_attention") val markAttention: Boolean = false,
    val notes: List<PlanViewNote> = emptyList(),
    @SerialName("waits_on_owner") val waitsOnOwner: Boolean = false,
    @SerialName("landed_landing") val landedLanding: String? = null,
    val agent: PlanViewAgentRef? = null,
    val run: PlanViewRun? = null,
    val branch: String = "",
    /** On a landing, the word beside the title: "Needs you" while it waits (which [notes] then leads with), "Landed · ac840108" once it has. */
    val note: String? = null,
    /** What a screen reader says for the row. */
    val accessibility: String = "",
)

@Serializable
enum class PlanViewWaitingKind {
    @SerialName("next") NEXT,
    @SerialName("not_ranked") NOT_RANKED,
    @SerialName("on_you") ON_YOU,
}

/** One of Waiting's groups; only groups with cards are sent. */
@Serializable
data class PlanViewWaitingGroup(
    val kind: PlanViewWaitingKind,
    /** Drawn only when another group shares the section. */
    val title: String = "",
    val attention: Boolean = false,
    val cards: List<PlanViewTicket> = emptyList(),
)

@Serializable
data class PlanViewLadderStep(val label: String = "", val explanation: String = "")

/** The ladder a landing climbs: three steps, and the one it stands on. */
@Serializable
data class PlanViewLadder(
    val steps: List<PlanViewLadderStep> = emptyList(),
    /** 0 to 2, or 3 once everything is done. */
    val at: Int = 0,
    /** "Checking on CI · step 2 of 3". */
    val compact: String = "",
)

@Serializable
data class PlanViewLanding(
    val id: String = "",
    val number: Int = 0,
    /** "Landing 86". */
    val label: String = "",
    val subtitle: String? = null,
    val carries: String? = null,
    val phase: String = "",
    /** "Checking on CI". */
    @SerialName("phase_word") val phaseWord: String = "",
    /** "checking on CI", inside a sentence. */
    @SerialName("phase_sentence") val phaseSentence: String = "",
    val explanation: String = "",
    /** CI failed, or nothing has moved. */
    val attention: Boolean = false,
    /** When it entered this phase, in ms. */
    val since: Long = 0,
    val ladder: PlanViewLadder = PlanViewLadder(),
    val lander: PlanViewAgentRef? = null,
    /** "Lander · Opus · 1.4M". */
    @SerialName("lander_words") val landerWords: String? = null,
    @SerialName("pushed_sha") val pushedSha: String? = null,
    @SerialName("ci_unread") val ciUnread: String? = null,
    val tickets: List<PlanViewTicket> = emptyList(),
)

@Serializable
data class PlanViewAgentRow(
    val id: String = "",
    val who: String = "",
    val working: Boolean = false,
    @SerialName("works_for") val worksFor: String = "",
    val state: String = "",
    /** "“Design ov-503” · ov-503 · Working · 22 min". */
    val second: String = "",
    @SerialName("doing_now") val doingNow: String = "",
    val tokens: String? = null,
    /** "≈$1.10". */
    val cost: String? = null,
    val terminal: String? = null,
    val accessibility: String = "",
)

@Serializable
data class PlanViewAgents(
    /** "5 agents, including the orchestrator · 4 working · 3.9M tokens in the last 24 hours · ≈$26". */
    val header: String = "",
    val rows: List<PlanViewAgentRow> = emptyList(),
)

/** A card's mark on a tile and in the sidebar: the site's three. */
@Serializable
enum class PlanCardGlyph {
    @SerialName("done") DONE,
    @SerialName("in_progress") IN_PROGRESS,
    @SerialName("not_started") NOT_STARTED;

    companion object {
        /** In review counts as in progress; waiting on a decision hasn't started, and the tile's ask line says who it waits on. */
        fun of(status: TaskStatus): PlanCardGlyph = when (status) {
            TaskStatus.DONE -> DONE
            TaskStatus.IN_PROGRESS, TaskStatus.IN_REVIEW -> IN_PROGRESS
            TaskStatus.BACKLOG, TaskStatus.TODO, TaskStatus.NEEDS_DECISION, TaskStatus.CANCELLED -> NOT_STARTED
        }
    }
}

@Serializable
data class PlanViewTileCard(
    val id: String = "",
    val key: String = "",
    val title: String = "",
    val glyph: PlanCardGlyph = PlanCardGlyph.NOT_STARTED,
)

@Serializable
data class PlanViewTile(
    val id: String = "",
    val name: String = "",
    val done: Int = 0,
    val total: Int = 0,
    /** "6 of 10 done". */
    val progress: String = "",
    /** How full the meter is, 0 to 1. */
    val fraction: Double = 0.0,
    val cards: List<PlanViewTileCard> = emptyList(),
    /** What waits on the owner: the one amber line. */
    @SerialName("owner_ask") val ownerAsk: String? = null,
    val moving: Boolean = false,
)

@Serializable
data class PlanViewThemes(
    /** Themes in motion first, then the rest by last move. */
    val tiles: List<PlanViewTile> = emptyList(),
    /** Done themes, folded into one tile. */
    val finished: List<PlanViewTile> = emptyList(),
    @SerialName("finished_title") val finishedTitle: String? = null,
    /** "7 themes". */
    @SerialName("finished_words") val finishedWords: String? = null,
)

/** The Plan's view model, decoded: every section to draw, in order, and the rows of each. */
@Serializable
data class PlanView(
    val schema: Int = 0,
    /** The runner's clock. */
    val now: Long = 0,
    val sections: List<PlanViewSection> = emptyList(),
    @SerialName("needs_you") val needsYou: Int = 0,
    /** Live landings, furthest along first. */
    val landings: List<PlanViewLanding> = emptyList(),
    @SerialName("in_review") val inReview: List<PlanViewTicket> = emptyList(),
    val building: List<PlanViewTicket> = emptyList(),
    /** Next Up ranked, Not Ranked, then Waiting on You; empty groups left out. */
    val waiting: List<PlanViewWaitingGroup> = emptyList(),
    @SerialName("on_landed_landing") val onLandedLanding: List<PlanViewTicket> = emptyList(),
    /** Landed on the viewer's day, newest first. */
    @SerialName("landed_today") val landedToday: List<PlanViewTicket> = emptyList(),
    val agents: PlanViewAgents = PlanViewAgents(),
    val themes: PlanViewThemes = PlanViewThemes(),
) {
    fun section(kind: PlanViewKind): PlanViewSection? = sections.firstOrNull { it.kind == kind }

    /** The cards in a Waiting group, or none. */
    fun waiting(kind: PlanViewWaitingKind): List<PlanViewTicket> = waiting.firstOrNull { it.kind == kind }.let { it?.cards ?: emptyList() }

    /** Every card Waiting holds. */
    val waitingCount: Int get() = waiting.sumOf { it.cards.size }

    companion object {
        private val json = Json { ignoreUnknownKeys = true; coerceInputValues = true; explicitNulls = false }

        /** The view in a bridge answer, or null for an `invalid` request or a reply that isn't one. */
        fun decode(answer: String?): PlanView? {
            answer ?: return null
            return try {
                val reply = json.parseToJsonElement(answer) as? JsonObject ?: return null
                val view = reply["view"] as? JsonObject ?: return null
                json.decodeFromJsonElement(serializer(), view)
            } catch (_: kotlinx.serialization.SerializationException) {
                null
            } catch (_: IllegalArgumentException) {
                null
            }
        }
    }
}

/**
 * Asks the shared core for the Plan's view. The request is the plan exactly as the runner sent it ([Plan.wire]), the
 * board's Needs You count and card statuses, and the workspace's workflow presentation when the app has it
 * ([Plan.workflow]). Null where the plan has no wire text (one built by hand), the core isn't in this build, or the core
 * says the request is invalid; the Plan then says it couldn't be read.
 */
object PlanViews {
    private val core: (String) -> String? = { request -> if (coreIsAvailable) NativeClient.nativePlanView(request) else null }

    /** The core's bridge; a test can stand its own in, and [reset] puts the core back. */
    internal var bridge: (String) -> String? = core

    private data class Key(val plan: Plan, val needsYou: Int, val statuses: Map<String, TaskStatus>, val activity: Map<String, Long>)

    private val cache = object : LinkedHashMap<Key, PlanView?>(8, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Key, PlanView?>?) = size > 6
    }

    /** Forget every view asked for, and ask the core again: for a test. */
    @Synchronized
    internal fun reset() {
        cache.clear()
        bridge = core
    }

    /** The context the core reads beside the plan. */
    internal fun context(needsYou: Int, statuses: Map<String, TaskStatus>, activity: Map<String, Long>): JsonObject = buildJsonObject {
        put("needs_you", maxOf(0, needsYou))
        put("statuses", buildJsonObject { for ((task, status) in statuses) put(task, status.wire) })
        put("activity", buildJsonObject { for ((task, at) in activity) put(task, at) })
        put("agents_supported", true)
        put("utc_offset", TimeZone.getDefault().getOffset(System.currentTimeMillis()) / 1000)
        put("style", "material")
    }

    /** The request for [plan] with the workflow presentation [workflow], or none. */
    internal fun request(plan: Plan, needsYou: Int, statuses: Map<String, TaskStatus>, activity: Map<String, Long>, withWorkflow: Boolean): String? {
        val wire = plan.wire ?: return null
        return buildString {
            append("{\"schema\":1,\"plan\":").append(wire)
            append(",\"context\":").append(context(needsYou, statuses, activity))
            plan.workflow?.takeIf { withWorkflow }?.let { append(",\"workflow\":").append(it) }
            append('}')
        }
    }

    @Synchronized
    fun of(
        plan: Plan,
        needsYou: Int = 0,
        statuses: Map<String, TaskStatus> = emptyMap(),
        activity: Map<String, Long> = emptyMap(),
    ): PlanView? {
        val key = Key(plan, needsYou, statuses, activity)
        if (cache.containsKey(key)) return cache[key]
        val view = ask(plan, needsYou, statuses, activity, withWorkflow = true)
            // A workflow presentation the core can't read must not take the plan with it.
            ?: if (plan.workflow != null) ask(plan, needsYou, statuses, activity, withWorkflow = false) else null
        cache[key] = view
        return view
    }

    private fun ask(plan: Plan, needsYou: Int, statuses: Map<String, TaskStatus>, activity: Map<String, Long>, withWorkflow: Boolean): PlanView? {
        val request = request(plan, needsYou, statuses, activity, withWorkflow) ?: return null
        return PlanView.decode(runCatching { bridge(request) }.getOrNull())
    }
}
