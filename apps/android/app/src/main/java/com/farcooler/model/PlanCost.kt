package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import java.util.Locale

// Cost on the plan (ov-307, folding in ov-261): a token budget on a theme or a
// lane, a theme's seven-day trend, the runner's tokens this week, and cost per
// finished card by harness and model. EXPERIMENTAL with the plan layer, behind
// `board_cost`: a runner without it sends none of this, and nothing here draws.
//
// The Android twin of AgentKit's `PlanCost.swift`, so a budget reads the same
// on the Mac, the iPhone and here, and pinned to the same fixture.
//
// Tokens come first and dollars second, and every dollar is API-equivalent. A
// budget is a number of tokens counted as a lane's spend counts them (input,
// output and both cache counts).
//
// **There is no weekly-limit percentage.** The runner can't know a plan's
// weekly limit: neither Claude Code nor Codex reports it, or how much of it is
// used, to the runner. The week is a count of tokens and says it has no limit.
// Never derive a percentage from a guessed limit.

/** What the finished cards a harness and model worked cost. */
data class PlanHarnessCost(
    val harness: String,
    /** Empty when the harness named no model. */
    val model: String,
    /**
     * Its share of the landed cards, in thousandths of a card: the n behind every figure here. A card two pairs
     * worked is one card in total, shared out by each pair's fraction of its tokens.
     */
    val cardShareMilli: Int,
    /** Tokens it spent on landed cards. */
    val tokens: Long,
    /** Millionths of a dollar it spent on landed cards; null unless every one of its turns was priced. */
    val costMicros: Long? = null,
) {
    val id: String get() = "$harness/$model"
}

/** The week and the comparison, as one plan read carries them. */
data class PlanCostRead(
    /** The runner's tokens over the last seven days, every harness and board. */
    val weekTokens: Long = 0,
    /** Harness and model pairs with enough finished cards to compare, most cards first. */
    val compare: List<PlanHarnessCost> = emptyList(),
    /** How many pairs had finished cards but too few to compare; the runner holds them back. */
    val compareHeldBack: Int = 0,
    /** Spend on the board's cards that haven't landed (open or cancelled), apart from the comparison. */
    val inFlightTokens: Long = 0,
    /** Millionths of a dollar of it; null unless every turn was priced. */
    val inFlightCostMicros: Long? = null,
) {
    /** Something to draw: tokens this week, a comparison, pairs held back, or spend in flight. */
    val isWorthShowing: Boolean get() = weekTokens > 0 || compare.isNotEmpty() || compareHeldBack > 0 || inFlightTokens > 0

    companion object {
        /** Null when the runner sent no `cost` (no `board_cost`). */
        fun decode(o: JsonObject?): PlanCostRead? = o?.let {
            PlanCostRead(
                weekTokens = it["week_tokens"]?.jsonPrimitive?.longOrNull ?: 0L,
                compare = (it["compare"] as? JsonArray).orEmpty().map { p ->
                    val c = p.jsonObject
                    PlanHarnessCost(
                        harness = c["harness"]?.jsonPrimitive?.content.orEmpty(),
                        model = c["model"]?.jsonPrimitive?.content.orEmpty(),
                        cardShareMilli = c["card_share_milli"]?.jsonPrimitive?.intOrNull ?: 0,
                        tokens = c["tokens"]?.jsonPrimitive?.longOrNull ?: 0L,
                        costMicros = c["cost_micros"]?.jsonPrimitive?.longOrNull,
                    )
                },
                compareHeldBack = it["compare_held_back"]?.jsonPrimitive?.intOrNull ?: 0,
                inFlightTokens = it["in_flight_tokens"]?.jsonPrimitive?.longOrNull ?: 0L,
                inFlightCostMicros = it["in_flight_cost_micros"]?.jsonPrimitive?.longOrNull,
            )
        }

        private fun JsonArray?.orEmpty(): JsonArray = this ?: JsonArray(emptyList())
    }
}

/** Where spend stands against a budget. Equal is within it. */
sealed interface PlanBudget {
    val used: Long
    val budget: Long
    val isOver: Boolean get() = this is Over

    data class Within(override val used: Long, override val budget: Long) : PlanBudget
    data class Over(override val used: Long, override val budget: Long) : PlanBudget
}

/** A theme's seven days as bars: each as a share of the busiest day. */
data class PlanTrend(val days: List<Long>) {
    val total: Long get() = days.sum()
    val peak: Long get() = days.maxOrNull() ?: 0L

    /** Each day's height from 0 to 1, against the busiest day. A day with anything in it never rounds away: it keeps a sliver. */
    fun heights(sliver: Double = 0.08): List<Double> =
        if (peak <= 0) days.map { 0.0 } else days.map { if (it == 0L) 0.0 else maxOf(sliver, it.toDouble() / peak) }
}

/** One row of the comparison: the pair, then what its finished cards cost. */
data class PlanCompareRow(
    val id: String,
    /** "Claude Code · opus". */
    val title: String,
    /** "5 finished cards · 1.5M tokens a card · about $2.50 a card". */
    val detail: String,
    /** For TalkBack: the same, with the numbers said in words. */
    val spoken: String,
)

/** The words cost draws in. Sentence case, as Material's copy is. */
object PlanCostWords {
    /** The fewest finished cards a harness and model need before the runner will compare them. */
    const val COMPARE_MINIMUM_CARDS = 3

    /** Said once under the week: which days, and why there is no percentage. */
    const val WEEK_NOTE =
        "Counted by UTC day. Your plan’s weekly limit isn’t something your runner can read, so this counts tokens and shows no percentage."

    /** Said under the comparison: what the numbers are. */
    const val COMPARE_NOTE =
        "Cost per landed card: what a harness and model spent on cards that landed, over its share of them. A card two models worked is one card, shared by their tokens. Dollars are API-equivalent."

    /** What is said where a dollar figure can't be: API-equivalent dollars the runner has no price for. */
    const val DOLLARS_NOT_REPORTED = "API-equivalent dollars: Not reported"

    /** A harness as a person names it. */
    fun harnessName(harness: String): String = when (harness) {
        "claude" -> "Claude Code"
        "codex" -> "Codex"
        "cursor" -> "Cursor"
        "opencode" -> "opencode"
        "" -> "Unknown"
        else -> harness.replaceFirstChar { it.uppercase() }
    }

    /** Where [spend] stands against [budget]; null without a budget. */
    fun budget(spend: PlanSpend?, against: Long?): PlanBudget? {
        if (against == null) return null
        val used = spend?.totalTokens ?: 0L
        return if (used > against) PlanBudget.Over(used, against) else PlanBudget.Within(used, against)
    }

    /** The warning a lane over its budget carries, in the words a row draws in amber; null when it has no budget or is within it. */
    fun overBudget(lane: PlanLane, locale: Locale = Locale.getDefault()): String? =
        (budget(lane.spend, lane.budgetTokens) as? PlanBudget.Over)?.let { budgetLine(it, locale) }

    /** The same, said for TalkBack with the counts as words. */
    fun overBudgetSpoken(lane: PlanLane, locale: Locale = Locale.getDefault()): String? =
        (budget(lane.spend, lane.budgetTokens) as? PlanBudget.Over)?.let { budgetSpoken(it, locale) }

    /** The same for a theme. */
    fun overBudget(theme: PlanTheme): PlanBudget.Over? = budget(theme.spend, theme.budgetTokens) as? PlanBudget.Over

    /** "1.2M of 5M tokens budgeted", or "Over budget: 6.1M of 5M tokens". */
    fun budgetLine(b: PlanBudget, locale: Locale = Locale.getDefault()): String {
        val used = TaskUsageFormat.tokens(b.used, locale)
        val budget = TaskUsageFormat.tokens(b.budget, locale)
        return if (b.isOver) "Over budget: $used of $budget tokens" else "$used of $budget tokens budgeted"
    }

    /** The same for TalkBack, with the counts said as words: "Over budget. 6.1 million of 5 million tokens used." */
    fun budgetSpoken(b: PlanBudget, locale: Locale = Locale.getDefault()): String {
        val used = spokenTokens(b.used, locale)
        val budget = spokenTokens(b.budget, locale)
        return if (b.isOver) "Over budget. $used of $budget tokens used." else "$used of $budget tokens budgeted. Within budget."
    }

    /** "6.1 million", "340 thousand", "812": a count a screen reader says whole rather than spelling a suffix letter. */
    fun spokenTokens(n: Long, locale: Locale = Locale.getDefault()): String {
        val short = TaskUsageFormat.tokens(n, locale)
        for ((suffix, word) in listOf("K" to " thousand", "M" to " million", "B" to " billion")) {
            if (short.endsWith(suffix)) return short.dropLast(1) + word
        }
        return short
    }

    /** The seven days as bars, or null when the runner sent none or all are empty: a chart of nothing says nothing. */
    fun trend(days: List<Long>?): PlanTrend? =
        days?.takeIf { it.size == 7 && it.any { d -> d > 0 } }?.let(::PlanTrend)

    /** "Last 7 days, oldest first: none, none, 40 thousand, ..., 160 thousand today. 320 thousand tokens in all." */
    fun trendSpoken(t: PlanTrend, locale: Locale = Locale.getDefault()): String {
        val each = t.days.map { if (it == 0L) "none" else spokenTokens(it, locale) }
        return "Last 7 days by UTC day, oldest first: ${each.dropLast(1).joinToString(", ")}, ${each.last()} today. " +
            "${spokenTokens(t.total, locale)} tokens in all."
    }

    /** "34M tokens in the last 7 days on this runner": the same seven UTC days as the trend, today so far. */
    fun week(tokens: Long, locale: Locale = Locale.getDefault()): String =
        "${TaskUsageFormat.tokens(tokens, locale)} tokens in the last 7 days on this runner"

    /** "4.2 finished cards", or "4 finished cards": a pair's share of the landed cards, with its decimal when it isn't whole. */
    fun cardShare(milli: Int): String =
        if (milli % 1000 == 0) "${milli / 1000} finished cards" else "${String.format(Locale.US, "%.1f", milli / 1000.0)} finished cards"

    /** One comparison row: cost per landed card is the pair's spend on landed cards over its share of them. */
    fun compareRow(p: PlanHarnessCost, locale: Locale = Locale.getDefault()): PlanCompareRow {
        val model = p.model.ifEmpty { "no model named" }
        val share = maxOf(p.cardShareMilli, 1)
        val cards = cardShare(p.cardShareMilli)
        val each = p.tokens * 1000 / share
        val detail = mutableListOf(cards, "${TaskUsageFormat.tokens(each, locale)} tokens a card")
        val spoken = mutableListOf(cards, "${spokenTokens(each, locale)} tokens a card")
        val micros = p.costMicros
        if (micros != null) {
            val dollars = TaskUsageFormat.dollars(micros * 1000 / share, locale)
            detail += "about $dollars a card API-equivalent"
            spoken += "about $dollars a card, API-equivalent"
        } else {
            detail += DOLLARS_NOT_REPORTED
            spoken += DOLLARS_NOT_REPORTED
        }
        val name = harnessName(p.harness)
        return PlanCompareRow(p.id, "$name · $model", detail.joinToString(" · "), "$name, $model. ${spoken.joinToString(", ")}")
    }

    /** "1.2M tokens on cards that haven't landed · about $9 API-equivalent", Not reported for the dollars; null with nothing in flight. */
    fun inFlight(cost: PlanCostRead, locale: Locale = Locale.getDefault()): String? {
        if (cost.inFlightTokens <= 0) return null
        val tokens = TaskUsageFormat.tokens(cost.inFlightTokens, locale)
        val dollars = cost.inFlightCostMicros?.let { "about ${TaskUsageFormat.dollars(it, locale)} API-equivalent" }
        return "$tokens tokens on cards that haven’t landed · ${dollars ?: DOLLARS_NOT_REPORTED}"
    }

    /** "2 other harness and model pairs held back until three cards have landed", or null. */
    fun heldBack(n: Int): String? {
        if (n <= 0) return null
        val pairs = if (n == 1) "1 other harness and model pair" else "$n other harness and model pairs"
        return "$pairs held back until three cards have landed"
    }
}
