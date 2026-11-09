package com.farcooler.model

import com.farcooler.core.NativeClient
import com.farcooler.core.coreIsAvailable
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

// The classes are generated from the Rust ones (`core/wire/WireTypes.kt`, ov-1035); these are the names the app has for them.
typealias PlanView = com.farcooler.core.wire.PlanView
typealias PlanViewKind = com.farcooler.core.wire.SectionKind
typealias PlanViewSection = com.farcooler.core.wire.Section
typealias PlanViewMark = com.farcooler.core.wire.Mark
typealias PlanViewNoteKind = com.farcooler.core.wire.NoteKind
typealias PlanViewNote = com.farcooler.core.wire.Note
typealias PlanViewAgentRef = com.farcooler.core.wire.AgentRef
typealias PlanViewGate = com.farcooler.core.wire.Gate
typealias PlanViewRun = com.farcooler.core.wire.RunStep
typealias PlanViewTicket = com.farcooler.core.wire.TicketCard
typealias PlanViewWaitingKind = com.farcooler.core.wire.WaitingKind
typealias PlanViewWaitingGroup = com.farcooler.core.wire.WaitingGroup
typealias PlanViewLadderStep = com.farcooler.core.wire.LadderStep
typealias PlanViewLadder = com.farcooler.core.wire.Ladder
typealias PlanViewLanding = com.farcooler.core.wire.LandingCard
typealias PlanViewAgentRow = com.farcooler.core.wire.AgentRow
typealias PlanViewAgents = com.farcooler.core.wire.Agents
typealias PlanCardGlyph = com.farcooler.core.wire.Glyph
typealias PlanViewTileCard = com.farcooler.core.wire.TileCard
typealias PlanViewTile = com.farcooler.core.wire.ThemeTile
typealias PlanViewThemes = com.farcooler.core.wire.Themes
typealias PlanViewDetailStage = com.farcooler.core.wire.DetailStage
typealias PlanViewTicketDetail = com.farcooler.core.wire.TicketDetail
typealias PlanViewTicketDetails = com.farcooler.core.wire.TicketDetails

/** A card's mark on a tile and in the sidebar: the site's three. In review counts as in progress; waiting on a decision
 *  hasn't started, and the tile's ask line says who it waits on. */
fun planCardGlyphOf(status: TaskStatus): PlanCardGlyph = when (status) {
    TaskStatus.DONE -> PlanCardGlyph.Done
    TaskStatus.IN_PROGRESS, TaskStatus.IN_REVIEW -> PlanCardGlyph.InProgress
    TaskStatus.BACKLOG, TaskStatus.TODO, TaskStatus.NEEDS_DECISION, TaskStatus.CANCELLED -> PlanCardGlyph.NotStarted
}

fun PlanView.section(kind: PlanViewKind): PlanViewSection? = sections.firstOrNull { it.kind == kind }

/** The cards in a Waiting group, or none. */
fun PlanView.waiting(kind: PlanViewWaitingKind): List<PlanViewTicket> = waiting.firstOrNull { it.kind == kind }.let { it?.cards ?: emptyList() }

/** Every card Waiting holds. */
val PlanView.waitingCount: Int get() = waiting.sumOf { it.cards.size }

fun PlanViewTicketDetails.ticket(task: String): PlanViewTicketDetail? = tickets.firstOrNull { it.id == task }

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

    private val json = Json { ignoreUnknownKeys = true; coerceInputValues = true; explicitNulls = false }

    /** The view in a bridge answer, or null for an `invalid` request or a reply that isn't one. */
    fun decode(answer: String?): PlanView? {
        answer ?: return null
        return try {
            val reply = json.parseToJsonElement(answer) as? JsonObject ?: return null
            val view = reply["view"] as? JsonObject ?: return null
            json.decodeFromJsonElement(PlanView.serializer(), view)
        } catch (_: kotlinx.serialization.SerializationException) {
            null
        } catch (_: IllegalArgumentException) {
            null
        }
    }

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

    /** A ticket's detail: its status, its pull request's stage and its stale warning, as the core says them. */
    fun detail(plan: Plan?, task: String): PlanViewTicketDetail? = plan?.let { of(it)?.details?.ticket(task) }

    /** A ticket's status line; its stage's own word when the core can't be asked. */
    fun status(plan: Plan?, ticket: PlanTicket): String = detail(plan, ticket.task)?.status ?: PlanWords.state(ticket)

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
        return decode(runCatching { bridge(request) }.getOrNull())
    }
}
