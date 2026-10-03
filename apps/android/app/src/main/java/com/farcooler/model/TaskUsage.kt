package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import java.text.NumberFormat
import java.util.Currency
import java.util.Locale
import kotlin.math.roundToLong

/**
 * What agents spent on one task (ov-195): its screen's Usage section.
 *
 * The runner's `usage.task` route answers in one shape,
 * `farcooler_core::usage_words::TaskSpend`, which the Mac reads too; this says it in
 * the same words AgentKit's `TaskUsageFormat` and `farcooler report` do. All three
 * are pinned by `test/fixtures/task-usage.json`.
 *
 * Every dollar is API-equivalent: the agent's own figure and the runner's price
 * table are both API list prices, notional on a subscription and what was paid on
 * an API key, and nothing here can tell which. A cost made partly from the table
 * says "estimated" or "partly estimated", one with tokens nobody can price says
 * "partly not reported", and a cost nobody knows is "Not reported", never a guess.
 */
data class TaskSpend(
    val turns: Long = 0,
    /** Turns whose tokens are a floor: some model call stated none. */
    val turnsPartial: Long = 0,
    /** Turns that stated no usage at all. */
    val turnsNotReported: Long = 0,
    val subagentRuns: Long = 0,
    val activeMs: Long = 0,
    val inputTokens: Long = 0,
    val outputTokens: Long = 0,
    val cacheReadTokens: Long = 0,
    val cacheWriteTokens: Long = 0,
    /** Millionths of a dollar the agents reported. */
    val costReportedMicros: Long = 0,
    /** Millionths of a dollar from the price table. */
    val costEstimatedMicros: Long = 0,
    /** Tokens with no known price. */
    val unpricedTokens: Long = 0,
) {
    val totalTokens: Long get() = inputTokens + outputTokens + cacheReadTokens + cacheWriteTokens
    /** Nothing recorded: no turn, and no subagent run. */
    val isEmpty: Boolean get() = turns == 0L && subagentRuns == 0L
    internal val pricedMicros: Long get() = costReportedMicros + costEstimatedMicros
    internal val partlyUnknown: Boolean get() = unpricedTokens > 0 || turnsPartial > 0 || turnsNotReported > 0

    companion object {
        fun decode(o: JsonObject?): TaskSpend {
            fun n(name: String) = o?.get(name)?.jsonPrimitive?.longOrNull ?: 0L
            return TaskSpend(
                turns = n("turns"),
                turnsPartial = n("turns_partial"),
                turnsNotReported = n("turns_not_reported"),
                subagentRuns = n("subagent_runs"),
                activeMs = n("active_ms"),
                inputTokens = n("input_tokens"),
                outputTokens = n("output_tokens"),
                cacheReadTokens = n("cache_read_tokens"),
                cacheWriteTokens = n("cache_write_tokens"),
                costReportedMicros = n("cost_reported_micros"),
                costEstimatedMicros = n("cost_estimated_micros"),
                unpricedTokens = n("unpriced_tokens"),
            )
        }
    }
}

/** One harness and model's share. [model] is empty when the harness named none. */
data class TaskSpendRow(val harness: String, val model: String, val totals: TaskSpend) {
    val title: String get() = if (model.isEmpty()) harness else "$harness · $model"
}

/** A task's spend: its totals and the same by harness and model. */
data class TaskUsage(
    val task: String,
    val priceTable: String,
    val totals: TaskSpend,
    val byHarnessModel: List<TaskSpendRow>,
) {
    /** The breakdown, most tokens first, then by title. */
    val rows: List<TaskSpendRow>
        get() = byHarnessModel.sortedWith(compareByDescending<TaskSpendRow> { it.totals.totalTokens }.thenBy { it.title })

    companion object {
        private val json = Json { ignoreUnknownKeys = true }

        fun decode(text: String): TaskUsage = decode(json.parseToJsonElement(text).jsonObject)

        fun decode(o: JsonObject): TaskUsage {
            fun text(name: String) = o[name]?.jsonPrimitive?.contentOrNull ?: ""
            val rows = (o["by_harness_model"] as? JsonArray).orEmpty().mapNotNull { element ->
                val r = element as? JsonObject ?: return@mapNotNull null
                TaskSpendRow(
                    harness = r["harness"]?.jsonPrimitive?.contentOrNull ?: "",
                    model = r["model"]?.jsonPrimitive?.contentOrNull ?: "",
                    totals = TaskSpend.decode(r["totals"] as? JsonObject),
                )
            }
            return TaskUsage(text("task"), text("price_table"), TaskSpend.decode(o["totals"] as? JsonObject), rows)
        }
    }
}

/** The words a Usage section says. */
object TaskUsageFormat {
    const val NOTHING_YET = "No agent usage recorded yet."
    const val NOT_REPORTED = "Not reported"
    /** What a runner too old to record spend gets. */
    const val NEEDS_UPDATE = "This runner needs an update to show spend."
    /** What a read that didn't come back gets, beside [TRY_AGAIN]. */
    const val COULDNT_READ = "Far Cooler couldn’t read this task’s usage."
    /** The button beside [COULDNT_READ]: Android's own label, in sentence case (ov-204). */
    const val TRY_AGAIN = "Try again"
    /** What "API-equivalent" means, said once under the spend total. */
    const val API_EQUIVALENT =
        "API-equivalent: what these tokens would cost at API list prices. On a subscription plan, you pay your plan’s price instead."

    /** Whether a cost line has a dollar figure, and so wants [API_EQUIVALENT] beneath it. */
    fun isPriced(s: TaskSpend): Boolean = s.pricedMicros > 0

    /**
     * A token count, short, in [locale]'s digits: "999", "1.2K", "12K", "1M",
     * "2.1B". One decimal below ten of a unit, none above; a count that rounds up
     * to a thousand of one unit is one of the next.
     */
    fun tokens(n: Long, locale: Locale = Locale.getDefault()): String {
        val format = NumberFormat.getNumberInstance(locale).apply { minimumFractionDigits = 0 }
        if (n < 1000) {
            format.maximumFractionDigits = 0
            return format.format(n)
        }
        val units = listOf(1e3 to "K", 1e6 to "M", 1e9 to "B")
        var unit = units.indexOfLast { n.toDouble() >= it.first }.coerceAtLeast(0)
        while (true) {
            val (scale, suffix) = units[unit]
            val value = n / scale
            val digits = if (value < 9.95) 1 else 0
            val power = if (digits == 1) 10.0 else 1.0
            val rounded = Math.round(value * power) / power
            if (rounded >= 1000 && unit + 1 < units.size) {
                unit += 1
                continue
            }
            format.maximumFractionDigits = digits
            return format.format(rounded) + suffix
        }
    }

    /** Millionths of a dollar in [locale]'s currency style; above zero and below a cent, "Under $0.01". */
    fun dollars(micros: Long, locale: Locale = Locale.getDefault()): String {
        val format = NumberFormat.getCurrencyInstance(locale).apply {
            currency = Currency.getInstance("USD")
            minimumFractionDigits = 2
            maximumFractionDigits = 2
        }
        if (micros in 1 until 10_000) return "Under " + format.format(0.01)
        val cents = (micros / 10_000.0).roundToLong()
        return format.format(cents / 100.0)
    }

    /** A total of agent time: "under a minute", "6 min", "3 h 10 min", and whole hours from ten on. */
    fun duration(ms: Long): String {
        val minutes = ms / 60_000
        return when {
            minutes < 1 -> "under a minute"
            minutes < 60 -> "$minutes min"
            minutes < 600 -> if (minutes % 60 == 0L) "${minutes / 60} h" else "${minutes / 60} h ${minutes % 60} min"
            else -> "${minutes / 60} h"
        }
    }

    /** "1.2M tokens", or "Not reported" when no turn stated any. */
    fun tokensLine(s: TaskSpend, locale: Locale = Locale.getDefault()): String =
        if (s.totalTokens == 0L) NOT_REPORTED else "${tokens(s.totalTokens, locale)} tokens"

    /** "12K input · 3.4K output · 1.1M cache", or null with no tokens. */
    fun tokenDetail(s: TaskSpend, locale: Locale = Locale.getDefault()): String? =
        if (s.totalTokens == 0L) null
        else listOf(
            "${tokens(s.inputTokens, locale)} input",
            "${tokens(s.outputTokens, locale)} output",
            "${tokens(s.cacheReadTokens + s.cacheWriteTokens, locale)} cache",
        ).joinToString(" · ")

    /**
     * "$3.20 · API-equivalent", with "estimated", "partly estimated" and "partly
     * not reported" as they apply; "Not reported" with no known cost.
     */
    fun cost(s: TaskSpend, locale: Locale = Locale.getDefault()): String {
        if (s.pricedMicros <= 0) return NOT_REPORTED
        val line = StringBuilder("${dollars(s.pricedMicros, locale)} · API-equivalent")
        if (s.costReportedMicros == 0L) line.append(", estimated")
        else if (s.costEstimatedMicros > 0) line.append(", partly estimated")
        if (s.partlyUnknown) line.append(", partly not reported")
        return line.toString()
    }

    /** "Agent time 3 h 10 min · 12 turns", either half alone, or null. */
    fun time(s: TaskSpend): String? {
        val parts = mutableListOf<String>()
        if (s.activeMs > 0) parts += "Agent time ${duration(s.activeMs)}"
        when (s.turns) {
            0L -> {}
            1L -> parts += "1 turn"
            else -> parts += "${s.turns} turns"
        }
        return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
    }

    /**
     * "1.2M tokens · $3.20", with "estimated", "partly estimated" and "partly not
     * reported" as they apply to the row's own part; "Cost not reported"; or "Not
     * reported" when the row stated nothing.
     */
    fun detail(row: TaskSpendRow, locale: Locale = Locale.getDefault()): String {
        val s = row.totals
        if (s.totalTokens == 0L && s.pricedMicros <= 0) return NOT_REPORTED
        val cost = if (s.pricedMicros <= 0) {
            "Cost not reported"
        } else {
            val words = buildList {
                if (s.costReportedMicros == 0L) add("estimated")
                else if (s.costEstimatedMicros > 0) add("partly estimated")
                if (s.partlyUnknown) add("partly not reported")
            }
            val amount = dollars(s.pricedMicros, locale)
            if (words.isEmpty()) amount else "$amount ${words.joinToString(", ")}"
        }
        return "${tokens(s.totalTokens, locale)} tokens · $cost"
    }
}

/** What a Usage section shows. */
sealed interface TaskUsageState {
    data object Loading : TaskUsageState
    /** The runner is older than spend: [TaskUsageFormat.NEEDS_UPDATE]. */
    data object NeedsUpdate : TaskUsageState
    /** The read didn't come back: [TaskUsageFormat.COULDNT_READ], with Try Again. */
    data object Failed : TaskUsageState
    data class Loaded(val usage: TaskUsage) : TaskUsageState

    companion object {
        /**
         * The state a read lands in. A runner that says it lacks `agent_usage`
         * ([runnerCan] false) needs an update; one whose build isn't known yet is
         * asked, and a refusal then reads as a failure.
         */
        fun after(usage: TaskUsage?, runnerCan: Boolean?): TaskUsageState = when {
            runnerCan == false -> NeedsUpdate
            usage != null -> Loaded(usage)
            else -> Failed
        }
    }
}
