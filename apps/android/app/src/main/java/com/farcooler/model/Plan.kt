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

// The plan layer on Android (ov-268 design 6.5, phase P5 is ov-274): themes
// (why a group of cards exists), lanes (one unit of execution) and the plan
// (the queued lanes, first is next up), as the phone's Plan view reads them.
// EXPERIMENTAL, behind the runner's `board_plan` capability, and removable:
// nothing on a task points here.
//
// The Android twin of AgentKit's `PlanModel.swift` and `PlanReading.swift`, so
// a lane reads the same on the Mac, the iPhone and here. The runner answers
// `plan.get` in the shape the CLI's `plan --json` prints; both are held to
// `test/fixtures/plan.json` by the Rust client's and the CLI's tests, and this
// file's `PlanTest` decodes the same file, so a key renamed on the wire fails
// there. Copy is Material's: sentence case.

/** A card a theme or lane names, by id and key; a lane's may name a slice. */
data class PlanCardRef(val task: String, val key: String, val slice: String = "")

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
    /** What its lanes spent on its cards, each lane's spend shared over its cards (ov-306); null from a runner before it. */
    val spend: PlanSpend? = null,
    /** Its token budget (ov-307), counted as [spend] is; null when it has none or from a runner without `board_cost`. */
    val budgetTokens: Long? = null,
    /** Its tokens on each of the last seven UTC days, oldest first and today last (ov-307); null from a runner without `board_cost`. */
    val trendTokens: List<Long>? = null,
    /** When it last moved by the runner's reckoning (its story, lanes, rulings and cards, ov-331); null from a runner before it. */
    val lastMovedAt: Long? = null,
)

/** Where a lane is. Moves go forward, with two loops back to fixing. */
enum class LaneState(val wire: String) {
    QUEUED("queued"), BUILDING("building"), REVIEW("review"), FIXING("fixing"), LANDING("landing"),
    LANDED("landed"), DROPPED("dropped"), UNKNOWN("unknown");

    /** Neither landed nor dropped. */
    val isLive: Boolean get() = this != LANDED && this != DROPPED

    companion object {
        /** A state this build has no word for is [UNKNOWN], not a failed read. */
        fun parse(word: String?): LaneState = entries.firstOrNull { it.wire == word } ?: UNKNOWN
    }
}

data class PlanAgent(
    val harness: String,
    val agentId: String,
    /** `build`, `review` or `fix`. */
    val role: String,
    val model: String,
    val startedAt: Long,
    val endedAt: Long?,
)

/**
 * What a lane's agents spent. An agent the runner has read no turn of is
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
    /** Agents also on another lane, whose spend is split evenly across their lanes: the figures hold this lane's part. */
    val sharedAgents: Int = 0,
) {
    val totalTokens: Long get() = inputTokens + outputTokens + cacheReadTokens + cacheWriteTokens
}

data class PlanLane(
    val id: String,
    val name: String,
    val state: LaneState,
    /** One line: why it's in the plan, or why it's where it is now. */
    val reason: String,
    /** 1 is next up; null outside the plan. */
    val planRank: Int?,
    val worktreePath: String,
    val branch: String,
    val harness: String,
    val model: String,
    val train: String?,
    val landedSha: String?,
    val stateSince: Long,
    /** Sat in one live state for over an hour, by the runner's clock. */
    val stale: Boolean,
    val fixRounds: Int,
    val cards: List<PlanCardRef>,
    val agents: List<PlanAgent>,
    val spend: PlanSpend,
    /** Its token budget (ov-307); null when it has none. */
    val budgetTokens: Long? = null,
    /**
     * What a person reads for it (ov-462): the title the orchestrator wrote, else one the runner derived from its
     * cards' titles. Null or empty from a runner before it, and for a lane with no cards and no title.
     */
    val title: String? = null,
) {
    /** What every surface shows first: the title, else the name. */
    val heading: String get() = title?.takeIf { it.isNotEmpty() } ?: name

    /** The slug, shown second where there is room: null when the heading is the name already. */
    val slug: String? get() = name.takeIf { heading != it }
}

/** A lane the plan flags because a live train has its name (ov-461): the integrating agent modeled as a lane. */
data class PlanFlaggedLane(val lane: String, val name: String)

/** A card's row, as the plan read it: for a card the board hasn't read. */
data class PlanCard(val task: String, val key: String, val title: String, val status: String)

/** One read of a board's plan: every Plan surface draws from it. */
data class Plan(
    /** The runner's clock when it answered. */
    val nowMs: Long = 0,
    val themes: List<PlanTheme> = emptyList(),
    /** Live lanes, and the ones that finished in the last week. */
    val lanes: List<PlanLane> = emptyList(),
    /** The queued lanes in the plan, by id; first is next up. */
    val order: List<String> = emptyList(),
    val cards: List<PlanCard> = emptyList(),
    /** Decided for you (ov-304): standing rulings newest first, then the settled ones, in the runner's order. */
    val rulings: List<PlanRuling> = emptyList(),
    /** Trains (ov-309): those not landed or dropped, oldest first, then the settled ones. */
    val trains: List<PlanTrain> = emptyList(),
    /** What the runner last read of CI for each subject the board names (ov-306). */
    val ci: List<PlanCiRead> = emptyList(),
    /** How many of the board's cards are in each status (ov-306); null from a runner before it. */
    val boardCounts: PlanCounts? = null,
    /** The week's tokens and the harness and model comparison (ov-307); null from a runner without `board_cost`. */
    val cost: PlanCostRead? = null,
    /** Live lanes named like a live train (ov-461): Now draws the train once, and "Worth a look" says so. */
    val laneIsTrain: List<PlanFlaggedLane> = emptyList(),
) {
    /** Nothing planned: no theme and no lane. Rulings don't count: they never switch a board's layout (review 1005a F1). */
    val isEmpty: Boolean get() = themes.isEmpty() && lanes.isEmpty()

    /** Nothing at all to show: nothing planned and no ruling. */
    val showsNothing: Boolean get() = isEmpty && rulings.isEmpty()

    /** Next up: the plan's queued lanes, first is next. */
    val nextUp: List<PlanLane>
        get() = order.mapNotNull { id -> lanes.firstOrNull { it.id == id && it.state == LaneState.QUEUED } }

    /** Now: the lanes being built, reviewed, fixed or landed, in the order they started. */
    val working: List<PlanLane> get() = lanes.filter { it.state.isLive && it.state != LaneState.QUEUED }

    /** Queued lanes the plan doesn't rank: written, but not yet ordered. */
    val unranked: List<PlanLane> get() = lanes.filter { it.state == LaneState.QUEUED && it.id !in order }

    /** The lanes that landed on the runner's day, in [zone], most recent first. */
    fun landedToday(zone: TimeZone = TimeZone.getDefault()): List<PlanLane> {
        fun day(ms: Long) = Calendar.getInstance(zone).apply { timeInMillis = ms }.let {
            it.get(Calendar.YEAR) * 1000 + it.get(Calendar.DAY_OF_YEAR)
        }
        val today = day(nowMs)
        return lanes.filter { it.state == LaneState.LANDED && day(it.stateSince) == today }
            .sortedByDescending { it.stateSince }
    }

    /** The themes as the overview lists them: active, then paused, then done, each by its order on the board. */
    val shownThemes: List<PlanTheme>
        get() {
            val rank = mapOf("active" to 0, "paused" to 1, "done" to 2)
            return themes.filter { it.state != "dropped" }
                .sortedWith(compareBy<PlanTheme>({ rank[it.state] ?: 3 }, { it.ordinal }))
        }

    /** The theme a lane serves: the one most of its cards are in, the earlier on the board when two tie. */
    fun themeOf(lane: PlanLane): PlanTheme? {
        var best: Pair<PlanTheme, Int>? = null
        for (theme in shownThemes) {
            val keys = theme.cards.map { it.task }.toSet()
            val count = lane.cards.count { it.task in keys }
            if (count > 0 && count > (best?.second ?: 0)) best = theme to count
        }
        return best?.first
    }

    /** The lanes working any of a theme's cards: live ones as listed, then queued in plan order, then the finished. */
    fun lanesIn(theme: PlanTheme): List<PlanLane> {
        val keys = theme.cards.map { it.task }.toSet()
        fun group(l: PlanLane) = when (l.state) {
            LaneState.QUEUED -> 1
            LaneState.LANDED, LaneState.DROPPED -> 2
            else -> 0
        }
        return lanes.withIndex().filter { (_, l) -> l.cards.any { it.task in keys } }
            .sortedWith { a, b ->
                val (ga, gb) = group(a.value) to group(b.value)
                when {
                    ga != gb -> ga.compareTo(gb)
                    ga == 1 -> compareValuesBy(a, b, { it.value.planRank ?: Int.MAX_VALUE }, { it.index })
                    ga == 2 -> b.value.stateSince.compareTo(a.value.stateSince)
                    else -> a.index.compareTo(b.index)
                }
            }.map { it.value }
    }

    /**
     * Whether a live lane is waiting on the owner: one of its cards needs a
     * decision, by the board's own statuses. With staleness, the only thing
     * that draws a lane in color.
     */
    fun waitsOnOwner(lane: PlanLane, statuses: Map<String, TaskStatus>): Boolean =
        lane.state.isLive && lane.cards.any { statuses[it.task] == TaskStatus.NEEDS_DECISION }

    fun card(task: String): PlanCard? = cards.firstOrNull { it.task == task }

    companion object {
        fun decode(text: String): Plan = decode(Json.parseToJsonElement(text).jsonObject)

        fun decode(o: JsonObject): Plan = Plan(
            nowMs = o.long("now_ms"),
            themes = o.list("themes").map { theme(it.jsonObject) },
            lanes = o.list("lanes").map { lane(it.jsonObject) },
            order = o.list("order").map { it.jsonPrimitive.content },
            cards = o.list("cards").map {
                val c = it.jsonObject
                PlanCard(c.string("task"), c.string("key"), c.string("title"), c.string("status"))
            },
            rulings = PlanRuling.decodeAll(o),
            trains = PlanTrain.decodeAll(o),
            ci = PlanCiRead.decodeAll(o),
            boardCounts = (o["board_counts"] as? JsonObject)?.let(::counts),
            cost = PlanCostRead.decode(o["cost"] as? JsonObject),
            laneIsTrain = o.list("lane_is_train").map { PlanFlaggedLane(it.jsonObject.string("lane"), it.jsonObject.string("name")) },
        )

        /** A status count object: a theme's, or the board's. */
        internal fun counts(c: JsonObject): PlanCounts {
            fun n(name: String) = c[name]?.jsonPrimitive?.intOrNull ?: 0
            return PlanCounts(n("backlog"), n("todo"), n("needs_decision"), n("in_progress"), n("in_review"), n("done"), n("cancelled"))
        }

        /** A spend object: a lane's, or a theme's share. */
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
            PlanCardRef(c.string("task"), c.string("key"), c.string("slice"))
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

        private fun lane(o: JsonObject): PlanLane {
            return PlanLane(
                id = o.string("id"), name = o.string("name"), state = LaneState.parse(o.maybe("state")),
                reason = o.string("reason"), planRank = o["plan_rank"]?.jsonPrimitive?.intOrNull,
                worktreePath = o.string("worktree_path"), branch = o.string("branch"),
                harness = o.string("harness"), model = o.string("model"), train = o.maybe("train"),
                landedSha = o.maybe("landed_sha"), stateSince = o.long("state_since"),
                stale = o["stale"]?.jsonPrimitive?.contentOrNull == "true", fixRounds = o.long("fix_rounds").toInt(),
                cards = refs(o),
                agents = o.list("agents").map {
                    val a = it.jsonObject
                    PlanAgent(
                        a.string("harness"), a.string("agent_id"), a.string("role"), a.string("model"),
                        a.long("started_at"), a["ended_at"]?.jsonPrimitive?.longOrNull,
                    )
                },
                spend = spend(o["spend"] as? JsonObject),
                budgetTokens = o["budget_tokens"]?.jsonPrimitive?.longOrNull,
                title = o.maybe("title"),
            )
        }
    }
}

private fun JsonObject.string(name: String): String = maybe(name) ?: ""
private fun JsonObject.maybe(name: String): String? =
    (this[name] as? JsonElement)?.takeIf { it !is JsonNull }?.jsonPrimitive?.contentOrNull
private fun JsonObject.long(name: String): Long = this[name]?.jsonPrimitive?.longOrNull ?: 0L
private fun JsonObject.list(name: String): JsonArray = this[name] as? JsonArray ?: JsonArray(emptyList())

/** One entry in a theme's or lane's record, oldest first. */
data class PlanEvent(val at: Long, val actor: String, val kind: String, val body: String)

/** One line of a theme's or lane's timeline. */
data class PlanTimelineRow(val at: Long, val text: String)

/** `plan.events`' answer: a theme's or lane's record. */
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

    /** The read was refused, failed, wasn't understood or timed out: [PlanWords.COULDNT_READ], with Try again. */
    data object Unavailable : PlanReadState

    data class Loaded(val plan: Plan) : PlanReadState

    companion object {
        /** How long a phone waits for `plan.get` before it says it couldn't. */
        const val TIMEOUT_MS = 15_000L

        /**
         * Read the plan, and land in one state whatever happens: the runner
         * not advertising `board_plan` (false) is [NeedsUpdate] without
         * asking, and null, a build not read yet, asks. An answer that never
         * comes is [Unavailable] after [timeoutMs], never a spinner for good.
         */
        suspend fun read(
            runnerCan: Boolean?,
            timeoutMs: Long = TIMEOUT_MS,
            isUnsupported: (Throwable) -> Boolean = { false },
            fetch: suspend () -> String,
        ): PlanReadState {
            if (runnerCan == false) return NeedsUpdate
            return try {
                withTimeoutOrNull(timeoutMs) { Loaded(Plan.decode(fetch())) } ?: Unavailable
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
        "The orchestrator writes the plan: themes say why a group of cards exists, and lanes are the agents working them."
    const val NEEDS_UPDATE = "This runner needs an update to keep a plan."
    const val COULDNT_READ = "Far Cooler couldn’t read this board’s plan."
    const val TRY_AGAIN = "Try again"
    const val NOT_REPORTED = TaskUsageFormat.NOT_REPORTED

    /** How many lines a theme's outcome gets before it truncates: the owner's ruling (ov-273). */
    const val OUTCOME_LINES = 3

    fun state(state: LaneState): String = when (state) {
        LaneState.QUEUED -> "Queued"
        LaneState.BUILDING -> "Building"
        LaneState.REVIEW -> "In review"
        LaneState.FIXING -> "Fixing"
        LaneState.LANDING -> "Landing"
        LaneState.LANDED -> "Landed"
        LaneState.DROPPED -> "Dropped"
        LaneState.UNKNOWN -> "Unknown"
    }

    /** A lane's state with what it needs said beside it: "Fixing · round 1", "Landing · in integ-8", "Queued · 2nd". */
    fun status(lane: PlanLane): String {
        val parts = mutableListOf(state(lane.state))
        if (lane.state == LaneState.FIXING && lane.fixRounds > 0) parts += "round ${lane.fixRounds}"
        lane.planRank?.let { if (lane.state == LaneState.QUEUED) parts += ordinal(it) }
        lane.landedSha?.let { if (lane.state == LaneState.LANDED && it.isNotEmpty()) parts += it.take(8) }
        lane.train?.let { if (lane.state.isLive && it.isNotEmpty()) parts += "in $it" }
        return parts.joinToString(" · ")
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
            parts += if (s.sharedAgents == 1) "1 agent's spend split with other lanes" else "${s.sharedAgents} agents' spend split with other lanes"
        }
        return parts.joinToString(" · ")
    }

    fun fixRounds(n: Int): String = if (n == 1) "1 fix round" else "$n fix rounds"

    /** "Builder Opus, 3 h": one agent, its role, model and how long it ran (to now while it's open). */
    fun agent(a: PlanAgent, now: Long): String {
        val role = when (a.role) {
            "review" -> "Reviewer"
            "fix" -> "Fixer"
            else -> "Builder"
        }
        val model = model(a.model)
        val ran = TaskUsageFormat.duration(maxOf(0, (a.endedAt ?: now) - a.startedAt))
        return if (model.isEmpty()) "$role, $ran" else "$role $model, $ran"
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

    /** A stale lane's warning, in words beside its mark: "No move in an hour". */
    fun stale(lane: PlanLane, now: Long): String? {
        if (!lane.stale) return null
        val minutes = maxOf(0, now - lane.stateSince) / 60_000
        return if (minutes < 120) "No move in an hour" else "No move in ${minutes / 60} h"
    }
}

/** One part of a theme's bar. */
data class PlanSegment(val kind: Kind, val count: Int) {
    enum class Kind { DONE, IN_REVIEW, IN_PROGRESS, NOT_STARTED }
}

/** A theme's or lane's page, by the plan's own id, or an orchestrator's page by its slot (ov-285). */
sealed interface PlanPage {
    val id: String

    data class Theme(override val id: String) : PlanPage
    data class Lane(override val id: String) : PlanPage

    /** An orchestrator's page (ov-269): [id] is its slot, `train` or `spend`. */
    data class Page(override val id: String) : PlanPage

    /** The word [Route.PlanPage] keeps it under. */
    val kind: String get() = when (this) {
        is Theme -> "theme"
        is Lane -> "lane"
        is Page -> "page"
    }

    companion object {
        fun of(kind: String, id: String): PlanPage = when (kind) {
            "theme" -> Theme(id)
            "page" -> Page(id)
            else -> Lane(id)
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
