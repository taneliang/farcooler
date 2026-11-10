package com.farcooler.model

import com.farcooler.core.wire.UsageSaid
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import java.text.NumberFormat
import java.util.Currency
import java.util.Locale
import kotlin.math.roundToLong

/**
 * What agents spent on one task (ov-195): its screen's Usage section.
 *
 * The client core answers `usage.task` with the runner's numbers and `said`, every
 * line the section draws (`farcooler_client::usage_json`, ov-1048), the same lines
 * the Mac, iOS and `farcooler task usage` show. This draws them.
 */
data class TaskUsage(val said: UsageSaid) {
    companion object {
        private val json = Json { ignoreUnknownKeys = true; explicitNulls = false }

        fun decode(text: String): TaskUsage = decode(json.parseToJsonElement(text).jsonObject)

        fun decode(o: JsonObject): TaskUsage = TaskUsage(json.decodeFromJsonElement(UsageSaid.serializer(), o["said"]!!))
    }
}

/** The words a Usage section says. */
object TaskUsageFormat {
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
}

/** What a Usage section shows. */
sealed interface TaskUsageState {
    data object Loading : TaskUsageState
    /** The runner is older than spend: [TaskUsageFormat.NEEDS_UPDATE]. */
    data object NeedsUpdate : TaskUsageState
    /** The read didn't come back: [TaskUsageFormat.COULDNT_READ], with Try again. */
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
