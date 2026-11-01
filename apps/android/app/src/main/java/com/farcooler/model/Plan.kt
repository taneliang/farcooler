package com.farcooler.model

import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import java.util.Calendar
import java.util.Locale
import java.util.TimeZone

// The plan layer on Android (ov-268 design 6.5, phase P5 is ov-274; ov-872): themes
// (why a group of cards exists), the line of tickets (each a card the plan means
// to build, with its stage), the agents working them and the landings that put
// them on main, as the phone's Plan view reads them.
// EXPERIMENTAL, behind the runner's `board_plan` capability, and removable:
// nothing on a task points here.
//
// The Android twin of AgentKit's `PlanModel.swift` and `PlanReading.swift`, so
// a ticket reads the same on the Mac, the iPhone and here. The runner answers
// `plan.get` in the shape the CLI's `plan --json` prints; both are held to
// `test/fixtures/plan.json` by the Rust client's and the CLI's tests, and this
// file's `PlanTest` decodes the same file, so a key renamed on the wire fails
// there. Copy is Material's: sentence case.

/** A card a theme, a ticket or a landing names, by id and key. */
data class PlanCardRef(
    val task: String,
    val key: String,
    /** Where this card's own pull request stands (ov-317); null for a card that has none to say. */
    val stage: PlanPrStage? = null,
)

/** How many of a theme's cards are in each of the board's statuses. */
data class PlanCounts(
    val backlog: Int = 0,
    val todo: Int = 0,
    val needsDecision: Int = 0,
    val inProgress: Int = 0,
    val inReview: Int = 0,
    val done: Int = 0,
    val cancelled: Int = 0,
)

data class PlanTheme(
    val id: String,
    val name: String,
    /** One sentence: the world when it's done. */
    val outcome: String,
    /** Where it stands, rewritten at checkpoints. */
    val story: String,
    /** When [story] was last written, in ms; 0 if never. */
    val storyAt: Long,
    val next: String,
    /** What needs the owner; empty when nothing does. */
    val ownerAsk: String,
    /** `active`, `paused`, `done` or `dropped`. */
    val state: String,
    val ordinal: Long,
    val cards: List<PlanCardRef>,
    val counts: PlanCounts,
    /** What its tickets spent, summed (ov-306); null from a runner before it. */
    val spend: PlanSpend? = null,
    /** Its token budget (ov-307), counted as [spend] is; null when it has none or from a runner without `board_cost`. */
    val budgetTokens: Long? = null,
    /** Its tokens on each of the last seven UTC days, oldest first and today last (ov-307); null from a runner without `board_cost`. */
    val trendTokens: List<Long>? = null,
    /** When it last moved by the runner's reckoning (its story, tickets, rulings and cards, ov-331); null from a runner before it. */
    val lastMovedAt: Long? = null,
)

/** Where a ticket is on the line. It moves forward, with a loop back to building for a fix round. */
enum class TicketStage(val wire: String) {
    WAITING("waiting"), BUILDING("building"), REVIEWING("reviewing"), NEEDS_YOU("needs_you"), LANDING("landing"),
    LANDED("landed"), DROPPED("dropped"), UNKNOWN("unknown");

    /** Neither landed nor dropped. */
    val isLive: Boolean get() = this != LANDED && this != DROPPED

    companion object {
        /** A stage this build has no word for is [UNKNOWN], not a failed read. */
        fun parse(word: String?): TicketStage = entries.firstOrNull { it.wire == word } ?: UNKNOWN
    }
}

/** What an agent does for one ticket. */
enum class WorkRole(val wire: String, val word: String) {
    BUILD("build", "Builder"), FIX("fix", "Fixer"), REVIEW("review", "Reviewer"), LAND("land", "Lander"), UNKNOWN("unknown", "Agent");

    companion object {
        fun parse(word: String?): WorkRole = entries.firstOrNull { it.wire == word } ?: UNKNOWN
    }
}

/** One agent's work on a ticket, oldest first on the ticket: who, doing what, since when and how it ended. */
data class PlanWork(
    /** The agent's id; the agent may no longer be in [Plan.agents]. */
    val agent: String,
    val role: WorkRole,
    val startedAt: Long,
    val endedAt: Long?,
    /** How it ended, in the agent's words; empty while it works. */
    val verdict: String = "",
) {
    val isOpen: Boolean get() = endedAt == null
}

/**
 * What a ticket's agents spent. An agent the runner has read no turn of is
 * [unmeasuredAgents], which reads "Not reported", never zero.
 */
data class PlanSpend(
    val inputTokens: Long = 0,
    val outputTokens: Long = 0,
    val cacheReadTokens: Long = 0,
    val cacheWriteTokens: Long = 0,
    /** Millionths of a dollar; null when no model's price is known. */
    val costMicros: Long? = null,
    val runs: Int = 0,
    val unmeasuredAgents: Int = 0,
    /** Agents also on another ticket, whose spend is split evenly across their tickets: the figures hold this ticket's part. */
    val sharedAgents: Int = 0,
) {
    val totalTokens: Long get() = inputTokens + outputTokens + cacheReadTokens + cacheWriteTokens
}

/** A ticket on the line (ov-872): a card the plan means to build, and how far it has got. */
data class PlanTicket(
    /** The card's task id. */
    val task: String,
    val key: String,
    val title: String,
    /** The card's board status, in the runner's word ("In Progress"). */
    val status: String,
    val stage: TicketStage,
    /** One line: why it's in the plan, or why it's where it is now. */
    val reason: String,
    /** 1 is next up; null outside the plan. */
    val planRank: Int?,
    val worktreePath: String,
    val branch: String,
    /** The ticket its branch is stacked on; it lands first. */
    val stackParent: PlanCardRef? = null,
    val prNumber: Int? = null,
    val landedSha: String?,
    val stageSince: Long,
    /** Taken off a landing by hand: the runner doesn't put it back on its own. */
    val heldOff: Boolean = false,
    /** Sat in one live stage for over an hour, by the runner's clock. */
    val stale: Boolean,
    val fixRounds: Int,
    /** The number of the live landing it is on, if any. */
    val landing: Int? = null,
    /** Everyone who worked it, oldest first. */
    val work: List<PlanWork> = emptyList(),
    val spend: PlanSpend = PlanSpend(),
    /** Its token budget (ov-307); null when it has none. */
    val budgetTokens: Long? = null,
    /** Where its pull request stands (ov-317); null with nothing to say. Read it through [shownStage]. */
    val prStage: PlanPrStage? = null,
) {
    /** The card it is, with its pull request's stage. */
    val card: PlanCardRef get() = PlanCardRef(task, key, prStage)

    /** What every surface shows first: the card's title, else its key. */
    val heading: String get() = title.ifEmpty { key }

    /** A fixer works it now: building, after a review. */
    val isFixing: Boolean get() = stage == TicketStage.BUILDING && work.any { it.role == WorkRole.FIX && it.isOpen }
}

/** A card's row, as the plan read it: for a card the board hasn't read. */
data class PlanCard(val task: String, val key: String, val title: String, val status: String)

/** One read of a board's plan: every Plan surface draws from it. */
data class Plan(
    /** The runner's clock when it answered. */
    val nowMs: Long = 0,
    val themes: List<PlanTheme> = emptyList(),
    /** The line (ov-872): tickets in the plan, live ones and the ones that landed in the last week. */
    val tickets: List<PlanTicket> = emptyList(),
    val cards: List<PlanCard> = emptyList(),
    /** Decided for you (ov-304): standing rulings newest first, then the settled ones, in the runner's order. */
    val rulings: List<PlanRuling> = emptyList(),
    /** Landings (ov-309, ov-872): those not landed or dropped, oldest first, then the settled ones. */
    val landings: List<PlanLanding> = emptyList(),
    /** What the runner last read of CI for each subject the board names (ov-306). */
    val ci: List<PlanCiRead> = emptyList(),
    /** How many of the board's cards are in each status (ov-306); null from a runner before it. */
    val boardCounts: PlanCounts? = null,
    /** The week's tokens and the harness and model comparison (ov-307); null from a runner without `board_cost`. */
    val cost: PlanCostRead? = null,
    /** Every agent that still exists, the orchestrator included (ov-503, ov-518, ov-872). */
    val agents: List<PlanAgent> = emptyList(),
    /** What every agent of the last 24 hours spent, those no longer in [agents] included; null from a runner before ov-518. */
    val agentsSpend: PlanSpend? = null,
) {
    /** Nothing planned: no theme and no ticket. Rulings don't count: they never switch a board's layout (review 1005a F1). */
    val isEmpty: Boolean get() = themes.isEmpty() && tickets.isEmpty()

    /** Nothing at all to show: nothing planned and no ruling. */
    val showsNothing: Boolean get() = isEmpty && rulings.isEmpty()

    /** Next up: the tickets waiting for a builder that the plan ranks, first is next. */
    val nextUp: List<PlanTicket>
        get() = tickets.filter { it.stage == TicketStage.WAITING && it.planRank != null }.sortedBy { it.planRank }

    /** Now: the tickets being built, reviewed, fixed or landed, in the order they started. */
    val working: List<PlanTicket> get() = tickets.filter { it.stage.isLive && it.stage != TicketStage.WAITING }

    /** Waiting tickets the plan doesn't rank: written, but not yet ordered. */
    val unranked: List<PlanTicket> get() = tickets.filter { it.stage == TicketStage.WAITING && it.planRank == null }

    /** The tickets that landed on the runner's day, in [zone], most recent first. */
    fun landedToday(zone: TimeZone = TimeZone.getDefault()): List<PlanTicket> {
        fun day(ms: Long) = Calendar.getInstance(zone).apply { timeInMillis = ms }.let {
            it.get(Calendar.YEAR) * 1000 + it.get(Calendar.DAY_OF_YEAR)
        }
        val today = day(nowMs)
        return tickets.filter { it.stage == TicketStage.LANDED && day(it.stageSince) == today }
            .sortedByDescending { it.stageSince }
    }

    /** The themes as the overview lists them: active, then paused, then done, each by its order on the board. */
    val shownThemes: List<PlanTheme>
        get() {
            val rank = mapOf("active" to 0, "paused" to 1, "done" to 2)
            return themes.filter { it.state != "dropped" }
                .sortedWith(compareBy<PlanTheme>({ rank[it.state] ?: 3 }, { it.ordinal }))
        }

    /** The theme a ticket serves: the first on the board whose cards hold it. */
    fun themeOf(ticket: PlanTicket): PlanTheme? = shownThemes.firstOrNull { theme -> theme.cards.any { it.task == ticket.task } }

    /** The tickets on any of a theme's cards: live ones as listed, then waiting in plan order, then the finished. */
    fun ticketsIn(theme: PlanTheme): List<PlanTicket> {
        val keys = theme.cards.map { it.task }.toSet()
        fun group(t: PlanTicket) = when (t.stage) {
            TicketStage.WAITING -> 1
            TicketStage.LANDED, TicketStage.DROPPED -> 2
            else -> 0
        }
        return tickets.withIndex().filter { (_, t) -> t.task in keys }
            .sortedWith { a, b ->
                val (ga, gb) = group(a.value) to group(b.value)
                when {
                    ga != gb -> ga.compareTo(gb)
                    ga == 1 -> compareValuesBy(a, b, { it.value.planRank ?: Int.MAX_VALUE }, { it.index })
                    ga == 2 -> b.value.stageSince.compareTo(a.value.stageSince)
                    else -> a.index.compareTo(b.index)
                }
            }.map { it.value }
    }

    /**
     * Whether a live ticket is waiting on the owner: it is at Needs you, or its card needs a decision, by the
     * board's own statuses. With staleness, the only thing that draws a ticket in color.
     */
    fun waitsOnOwner(ticket: PlanTicket, statuses: Map<String, TaskStatus>): Boolean =
        ticket.stage.isLive && (ticket.stage == TicketStage.NEEDS_YOU || statuses[ticket.task] == TaskStatus.NEEDS_DECISION)

    fun ticket(task: String): PlanTicket? = tickets.firstOrNull { it.task == task }

    /** The agents working a ticket now: those whose tickets name it, working ones first. */
    fun agentsOn(ticket: PlanTicket): List<PlanAgent> =
        agents.filter { a -> a.role != AgentRole.ORCHESTRATOR && a.tickets.any { it.task == ticket.task } }
            .sortedWith(compareByDescending<PlanAgent> { it.working }.thenByDescending { it.spend.totalTokens })

    fun card(task: String): PlanCard? = cards.firstOrNull { it.task == task }

    companion object {
        /** [PlanDecodeException] where [text] isn't a plan: the path of the field that isn't the shape it should be. */
        fun decode(text: String): Plan {
            val root = try {
                Json.parseToJsonElement(text)
            } catch (e: kotlinx.serialization.SerializationException) {
                throw PlanDecodeException("$")
            }
            return decode(root as? JsonObject ?: throw PlanDecodeException("$"))
        }

        fun decode(o: JsonObject): Plan = Plan(
            nowMs = DecodePath.at(".now_ms") { o.long("now_ms") },
            themes = DecodePath.at(".themes") { o.list("themes").mapIndexed { i, e -> DecodePath.at("[$i]") { theme(e.jsonObject) } } },
            tickets = DecodePath.at(".lines") { o.list("lines").mapIndexed { i, e -> DecodePath.at("[$i]") { ticket(e.jsonObject) } } },
            cards = DecodePath.at(".cards") {
                o.list("cards").mapIndexed { i, e ->
                    DecodePath.at("[$i]") {
                        val c = e.jsonObject
                        PlanCard(c.string("task"), c.string("key"), c.string("title"), c.string("status"))
                    }
                }
            },
            rulings = DecodePath.at(".rulings") { PlanRuling.decodeAll(o) },
            landings = DecodePath.at(".landings") { PlanLanding.decodeAll(o) },
            ci = DecodePath.at(".ci") { PlanCiRead.decodeAll(o) },
            boardCounts = DecodePath.at(".board_counts") { (o["board_counts"] as? JsonObject)?.let(::counts) },
            cost = DecodePath.at(".cost") { PlanCostRead.decode(o["cost"] as? JsonObject) },
            agents = DecodePath.at(".agents") { PlanAgent.decodeAll(o) },
            agentsSpend = DecodePath.at(".agents_spend") { (o["agents_spend"] as? JsonObject)?.let(::spend) },
        )

        /** A status count object: a theme's, or the board's. */
        internal fun counts(c: JsonObject): PlanCounts {
            fun n(name: String) = c[name]?.jsonPrimitive?.intOrNull ?: 0
            return PlanCounts(n("backlog"), n("todo"), n("needs_decision"), n("in_progress"), n("in_review"), n("done"), n("cancelled"))
        }

        /** A spend object: a ticket's, an agent's, or a theme's sum. */
        internal fun spend(s: JsonObject?): PlanSpend {
            fun n(name: String) = s?.get(name)?.jsonPrimitive?.longOrNull ?: 0L
            return PlanSpend(
                n("input_tokens"), n("output_tokens"), n("cache_read_tokens"), n("cache_write_tokens"),
                s?.get("cost_micros")?.jsonPrimitive?.longOrNull, n("runs").toInt(), n("unmeasured_agents").toInt(),
                n("shared_agents").toInt(),
            )
        }

        private fun refs(o: JsonObject) = o.list("cards").map {
            val c = it.jsonObject
            PlanCardRef(c.string("task"), c.string("key"))
        }

        private fun theme(o: JsonObject): PlanTheme = PlanTheme(
            id = o.string("id"), name = o.string("name"), outcome = o.string("outcome"),
            story = o.string("story"), storyAt = o.long("story_at"), next = o.string("next"),
            ownerAsk = o.string("owner_ask"), state = o.string("state"), ordinal = o.long("ordinal"),
            cards = refs(o),
            counts = counts(o["counts"] as? JsonObject ?: JsonObject(emptyMap())),
            spend = (o["spend"] as? JsonObject)?.let(::spend),
            budgetTokens = o["budget_tokens"]?.jsonPrimitive?.longOrNull,
            trendTokens = (o["trend_tokens"] as? JsonArray)?.map { it.jsonPrimitive.longOrNull ?: 0L },
            lastMovedAt = o["last_moved_at"]?.jsonPrimitive?.longOrNull,
        )

        private fun ticket(o: JsonObject): PlanTicket = PlanTicket(
            task = o.string("task"), key = o.string("key"), title = o.string("title"), status = o.string("status"),
            stage = TicketStage.parse(o.maybe("stage")), reason = o.string("reason"),
            planRank = o["plan_rank"]?.jsonPrimitive?.intOrNull,
            worktreePath = o.string("worktree_path"), branch = o.string("branch"),
            stackParent = (o["stack_parent"] as? JsonObject)?.let { PlanCardRef(it.string("task"), it.string("key")) },
            prNumber = o["pr_number"]?.jsonPrimitive?.intOrNull,
            landedSha = o.maybe("landed_sha"), stageSince = o.long("stage_since"),
            heldOff = o["held_off"]?.jsonPrimitive?.contentOrNull == "true",
            stale = o["stale"]?.jsonPrimitive?.contentOrNull == "true", fixRounds = o.long("fix_rounds").toInt(),
            landing = o["landing"]?.jsonPrimitive?.intOrNull,
            work = o.list("work").map {
                val w = it.jsonObject
                PlanWork(
                    w.string("agent"), WorkRole.parse(w.maybe("role")), w.long("started_at"),
                    w["ended_at"]?.jsonPrimitive?.longOrNull, w.string("verdict"),
                )
            },
            spend = spend(o["spend"] as? JsonObject),
            budgetTokens = o["budget_tokens"]?.jsonPrimitive?.longOrNull,
            prStage = PlanPrStage.decode(o["pr_stage"]),
        )
    }
}

private fun JsonObject.string(name: String): String = maybe(name) ?: ""
private fun JsonObject.maybe(name: String): String? =
    (this[name] as? JsonElement)?.takeIf { it !is JsonNull }?.let { primitive(it, name).contentOrNull }
private fun JsonObject.long(name: String): Long = this[name]?.let { primitive(it, name).longOrNull } ?: 0L

/** [element] as the primitive the field [name] should be; where it isn't, the exception names the field and says nothing of what it held. */
private fun primitive(element: JsonElement, name: String): kotlinx.serialization.json.JsonPrimitive =
    element as? kotlinx.serialization.json.JsonPrimitive ?: throw PlanDecodeException(DecodePath.path(name))

/** Where a plan didn't decode, as a path ("$.lines[2].stage"): keys and positions only, never a value (ov-631). */
class PlanDecodeException(val path: String) : Exception("the plan didn't decode at $path")

/** What is logged for a plan that didn't decode: the failing path where there is one, else the error's class; never its message, which can quote the plan. */
fun planDecodeLogText(e: Throwable): String =
    if (e is PlanDecodeException) "the plan didn't decode at ${e.path}" else "the plan didn't decode: ${e.javaClass.simpleName}"

/** The path to the field being read, kept while [Plan.decode] runs so a failure can say where. */
internal object DecodePath {
    private val stack = ThreadLocal.withInitial { ArrayList<String>() }

    fun path(leaf: String? = null): String = "$" + stack.get().joinToString("") + (leaf?.let { ".$it" } ?: "")

    /** Run [block] at [segment]; a cast or parse failure inside it becomes a [PlanDecodeException] at the innermost segment. */
    fun <T> at(segment: String, block: () -> T): T {
        val here = stack.get()
        here.add(segment)
        try {
            return block()
        } catch (e: PlanDecodeException) {
            throw e
        } catch (e: IllegalArgumentException) {
            throw PlanDecodeException(path())
        } finally {
            here.removeAt(here.size - 1)
        }
    }
}
private fun JsonObject.list(name: String): JsonArray = this[name] as? JsonArray ?: JsonArray(emptyList())

/** One entry in a theme's or ticket's record, oldest first. */
data class PlanEvent(val at: Long, val actor: String, val kind: String, val body: String)

/** One line of a theme's or ticket's timeline. */
data class PlanTimelineRow(val at: Long, val text: String)

/** `plan.events`' answer: a theme's or ticket's record. */
data class PlanRecord(val events: List<PlanEvent>) {
    /** The record as a timeline: a run of cards added (or taken out) in one write said once, and a story rewrite said as one. */
    val timeline: List<PlanTimelineRow>
        get() {
            val rows = mutableListOf<PlanTimelineRow>()
            var run: Triple<Long, String, MutableList<String>>? = null
            fun flush() {
                run?.let { (at, verb, keys) -> rows += PlanTimelineRow(at, cardsSentence(verb, keys)) }
                run = null
            }
            for (event in events) {
                val change = if (event.kind == "cards") cardChange(event.body) else null
                if (change != null) {
                    val (verb, key) = change
                    val open = run
                    if (open != null && open.second == verb && event.at - open.first < 60_000) open.third += key
                    else {
                        flush()
                        run = Triple(event.at, verb, mutableListOf(key))
                    }
                    continue
                }
                flush()
                rows += PlanTimelineRow(event.at, if (event.kind == "story") "Rewrote where it stands." else event.body)
            }
            flush()
            return rows
        }

    /** The story the last rewrite replaced, and when: "What changed". Null before the first rewrite. */
    val previousStory: Pair<String, Long>?
        get() = events.lastOrNull { it.kind == "story" }?.takeIf { it.body.isNotEmpty() }?.let { it.body to it.at }

    companion object {
        fun decode(text: String): PlanRecord = decode(Json.parseToJsonElement(text).jsonObject)

        fun decode(o: JsonObject) = PlanRecord(
            o.list("events").map {
                val e = it.jsonObject
                PlanEvent(e.long("at"), e.string("actor"), e.string("kind"), e.string("body"))
            },
        )

        /** "Added ov-155." gives ("Added", "ov-155"). */
        internal fun cardChange(body: String): Pair<String, String>? {
            val words = body.split(" ", limit = 2)
            if (words.size != 2 || words[0] !in listOf("Added", "Removed") || !words[1].endsWith(".")) return null
            val key = words[1].dropLast(1)
            return if (" " in key) null else words[0] to key
        }

        internal fun cardsSentence(verb: String, keys: List<String>): String = when (keys.size) {
            1 -> "$verb ${keys[0]}."
            2 -> "$verb ${keys[0]} and ${keys[1]}."
            3 -> "$verb ${keys[0]}, ${keys[1]} and ${keys[2]}."
            else -> "$verb ${keys[0]}, ${keys[1]} and ${keys.size - 2} more cards."
        }
    }
}

/** What a phone's Plan view is showing. */
sealed interface PlanReadState {
    /** A read is out and hasn't come back yet. */
    data object Loading : PlanReadState

    /** The runner is older than the plan layer: [PlanWords.NEEDS_UPDATE]. */
    data object NeedsUpdate : PlanReadState

    /** The read was refused, failed or wasn't understood: [PlanWords.COULDNT_READ], with Try again. */
    data object Unavailable : PlanReadState

    /**
     * The read got no answer because the link is down, as it is while the runner restarts (ov-631):
     * [PlanWords.RECONNECTING], not a plan that can't be read. The resync that follows the link reads again.
     */
    data object Reconnecting : PlanReadState

    data class Loaded(val plan: Plan) : PlanReadState

    companion object {
        /** How long a phone waits for `plan.get` before it says it couldn't. */
        const val TIMEOUT_MS = 15_000L

        /**
         * Read the plan, and land in one state whatever happens: the runner
         * not advertising `board_plan` (false) is [NeedsUpdate] without
         * asking, and null, a build not read yet, asks. An answer that never
         * comes is [Reconnecting] after [timeoutMs], never a spinner for good.
         */
        suspend fun read(
            runnerCan: Boolean?,
            timeoutMs: Long = TIMEOUT_MS,
            isUnsupported: (Throwable) -> Boolean = { false },
            isNoAnswer: (Throwable) -> Boolean = { false },
            onDecodeFailure: (Throwable) -> Unit = {},
            fetch: suspend () -> String,
        ): PlanReadState {
            if (runnerCan == false) return NeedsUpdate
            return try {
                withTimeoutOrNull(timeoutMs) {
                    val text = try {
                        fetch()
                    } catch (e: kotlinx.coroutines.CancellationException) {
                        throw e
                    } catch (e: Throwable) {
                        return@withTimeoutOrNull if (isUnsupported(e)) NeedsUpdate else if (isNoAnswer(e)) Reconnecting else Unavailable
                    }
                    try {
                        Loaded(Plan.decode(text))
                    } catch (e: kotlinx.coroutines.CancellationException) {
                        throw e
                    } catch (e: Throwable) {
                        onDecodeFailure(e)
                        Unavailable
                    }
                } ?: Reconnecting
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Throwable) {
                if (isUnsupported(e)) NeedsUpdate else Unavailable
            }
        }
    }
}

/** What the Plan view says. Sentence case, as Material's copy is; no parenthesized counts. */
object PlanWords {
    const val NOTHING_PLANNED = "Nothing is planned on this board yet."
    const val NOTHING_PLANNED_DETAIL =
        "The orchestrator writes the plan: themes say why a group of cards exists, tickets are the cards lined up to build, and agents work them."
    const val NEEDS_UPDATE = "This runner needs an update to keep a plan."
    const val COULDNT_READ = "Far Cooler couldn’t read this board’s plan."

    /** A read that got no answer because the runner is restarting or the link is down (ov-631). */
    const val RECONNECTING = "Reconnecting to the runner…"
    const val TRY_AGAIN = "Try again"
    const val NOT_REPORTED = TaskUsageFormat.NOT_REPORTED
    const val NOT_ON_A_TICKET = "Not on a ticket"
    const val NO_AGENTS = "No agents running."

    /** How many lines a theme's outcome gets before it truncates: the owner's ruling (ov-273). */
    const val OUTCOME_LINES = 3

    fun state(stage: TicketStage): String = when (stage) {
        TicketStage.WAITING -> "Waiting"
        TicketStage.BUILDING -> "Building"
        TicketStage.REVIEWING -> "In review"
        TicketStage.NEEDS_YOU -> "Needs you"
        TicketStage.LANDING -> "Landing"
        TicketStage.LANDED -> "Landed"
        TicketStage.DROPPED -> "Dropped"
        TicketStage.UNKNOWN -> "Unknown"
    }

    /** A ticket's stage, "Fixing" while a fixer builds it again after a review. */
    fun state(ticket: PlanTicket): String = if (ticket.isFixing) "Fixing" else state(ticket.stage)

    /**
     * A ticket's stage with what it needs said beside it: "Fixing · round 1", "Landing · in Landing 8", "Waiting · 2nd".
     * A pull request's stage says more than "In review" or "Landing", so it takes the stage's place (ov-317); one the
     * runner could not read is added, for it says less than the stage does. With [stage] false a row that draws the
     * pull request's stage on its own line keeps only what that stage leaves to the ticket ("Approved" beside "Checks failing").
     */
    fun status(ticket: PlanTicket, stage: Boolean = true): String {
        val parts = mutableListOf(state(ticket))
        ticket.shownStage?.let { shown ->
            val words = shown.fullWords
            if (words != null) {
                if (!stage) shown.stateWords?.let { parts[0] = it }
                else if (shown.kind == PlanPrStageKind.UNKNOWN) parts += words
                else parts[0] = words
            }
        }
        if (ticket.isFixing && ticket.fixRounds > 0) parts += "round ${ticket.fixRounds}"
        ticket.planRank?.let { if (ticket.stage == TicketStage.WAITING) parts += ordinal(it) }
        ticket.landedSha?.let { if (ticket.stage == TicketStage.LANDED && it.isNotEmpty()) parts += it.take(8) }
        ticket.landing?.let { if (ticket.stage.isLive) parts += "in ${PlanLanding.label(it)}" }
        return parts.joinToString(" · ")
    }

    /** [text] cut at a word to [max] characters, with an ellipsis: a title in the strip's one line. */
    fun short(text: String, max: Int = 24): String {
        if (text.length <= max) return text
        val cut = text.take(max - 1)
        val head = cut.substringBeforeLast(' ', cut).takeIf { it.length > max / 2 } ?: cut
        return head.trimEnd() + "\u2026"
    }

    /** "1st", "2nd", "3rd", "4th", … "11th", "12th", "13th", "21st". */
    fun ordinal(n: Int): String {
        if (n % 100 in 11..13) return "${n}th"
        return when (n % 10) {
            1 -> "${n}st"
            2 -> "${n}nd"
            3 -> "${n}rd"
            else -> "${n}th"
        }
    }

    fun cards(n: Int): String = if (n == 1) "1 card" else "$n cards"

    /** A model as people say it: "opus" and "claude-opus-5-5" are "Opus". */
    fun model(raw: String): String {
        val lower = raw.lowercase()
        for (family in listOf("opus", "sonnet", "haiku", "fable")) {
            if (family in lower) return family.replaceFirstChar { it.uppercase() }
        }
        return raw
    }

    /** The cards a theme's progress counts: all but the canceled. */
    fun total(c: PlanCounts): Int = c.backlog + c.todo + c.needsDecision + c.inProgress + c.inReview + c.done

    /** "4 of 18 done". Canceled cards count on neither side: they were neither left to do nor done. */
    fun progress(c: PlanCounts): String = "${c.done} of ${total(c)} done"

    /** The theme's bar, left to right: done, in review, in progress, then what hasn't started. Empty parts are left out. */
    fun segments(c: PlanCounts): List<PlanSegment> = listOf(
        PlanSegment(PlanSegment.Kind.DONE, c.done),
        PlanSegment(PlanSegment.Kind.IN_REVIEW, c.inReview),
        PlanSegment(PlanSegment.Kind.IN_PROGRESS, c.inProgress),
        PlanSegment(PlanSegment.Kind.NOT_STARTED, c.needsDecision + c.todo + c.backlog),
    ).filter { it.count > 0 }

    /** "3 done · 1 in progress · 6 backlog": a theme's cards by status, for its page. */
    fun breakdown(c: PlanCounts): String = listOf(
        c.done to "done", c.inReview to "in review", c.inProgress to "in progress",
        c.needsDecision to "need a decision", c.todo to "to do", c.backlog to "backlog",
    ).filter { it.first > 0 }.joinToString(" · ") { "${it.first} ${it.second}" }

    /** "470K tokens", with "about $31 estimated" when a price is known and "2 agents not reported"; "Not reported" with no tokens. */
    fun spend(s: PlanSpend, locale: Locale = Locale.getDefault()): String {
        if (s.totalTokens <= 0) return NOT_REPORTED
        val parts = mutableListOf("${TaskUsageFormat.tokens(s.totalTokens, locale)} tokens")
        s.costMicros?.takeIf { it > 0 }?.let { parts += "about ${TaskUsageFormat.dollars(it, locale)} estimated" }
        if (s.unmeasuredAgents > 0) {
            parts += if (s.unmeasuredAgents == 1) "1 agent not reported" else "${s.unmeasuredAgents} agents not reported"
        }
        if (s.sharedAgents > 0) {
            parts += if (s.sharedAgents == 1) "1 agent's spend split with other tickets" else "${s.sharedAgents} agents' spend split with other tickets"
        }
        return parts.joinToString(" · ")
    }

    fun fixRounds(n: Int): String = if (n == 1) "1 fix round" else "$n fix rounds"

    /** "Builder Opus, 3 h": one agent's work on a ticket, its role, model and how long it ran (to now while it's open). [model] is the agent's, when the plan still lists it. */
    fun work(w: PlanWork, now: Long, model: String = ""): String {
        val named = model(model)
        val ran = TaskUsageFormat.duration(maxOf(0, (w.endedAt ?: now) - w.startedAt))
        return if (named.isEmpty()) "${w.role.word}, $ran" else "${w.role.word} $named, $ran"
    }

    /** How long ago, coarsely: "just now", "5 min ago", "3 h ago", "2 d ago". */
    fun ago(ms: Long, now: Long): String {
        val minutes = maxOf(0, now - ms) / 60_000
        return when {
            minutes == 0L -> "just now"
            minutes < 60 -> "$minutes min ago"
            minutes < 1440 -> "${minutes / 60} h ago"
            else -> "${minutes / 1440} d ago"
        }
    }

    /** A stale ticket's warning, in words beside its mark: "No move in an hour". */
    fun stale(ticket: PlanTicket, now: Long): String? {
        if (!ticket.stale) return null
        val minutes = maxOf(0, now - ticket.stageSince) / 60_000
        return if (minutes < 120) "No move in an hour" else "No move in ${minutes / 60} h"
    }
}

/** One part of a theme's bar. */
data class PlanSegment(val kind: Kind, val count: Int) {
    enum class Kind { DONE, IN_REVIEW, IN_PROGRESS, NOT_STARTED }
}

/** A theme's or ticket's page, by the plan's own id, or an orchestrator's page by its slot (ov-285). */
sealed interface PlanPage {
    val id: String

    data class Theme(override val id: String) : PlanPage
    /** A ticket's page: [id] is its card's task id. */
    data class Ticket(override val id: String) : PlanPage

    /** An orchestrator's page (ov-269): [id] is its slot, `landing` or `spend`. */
    data class Page(override val id: String) : PlanPage

    /** The word [Route.PlanPage] keeps it under. */
    val kind: String get() = when (this) {
        is Theme -> "theme"
        is Ticket -> "ticket"
        is Page -> "page"
    }

    companion object {
        fun of(kind: String, id: String): PlanPage = when (kind) {
            "theme" -> Theme(id)
            "page" -> Page(id)
            else -> Ticket(id)
        }
    }
}

/** The client core's `plan` notice: that a board's plan was written, naming the board. */
object PlanNews {
    /** The board a `{"event": "plan", "workspace": ...}` line names, or null for any other line. */
    fun board(line: JsonObject): String? {
        if (line["event"]?.jsonPrimitive?.contentOrNull != "plan") return null
        return line["workspace"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotEmpty() }
    }
}

/** The Tasks | Plan choice, kept per board on this phone. Tasks until someone picks Plan. */
object PlanChoice {
    fun key(host: String, workspace: String) = "board.plan.shown.$host.$workspace"

    /**
     * What the board draws: the plan, only where the runner keeps one and
     * someone chose it. A runner without `board_plan` draws its tasks, and no
     * control, whatever was chosen before.
     */
    fun showing(runnerKeepsPlan: Boolean, chosen: Boolean): Boolean = runnerKeepsPlan && chosen
}
