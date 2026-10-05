package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull

// Decided for you (ov-304): the reversible calls the orchestrator made on the
// owner's behalf, read with the plan (`plan.get`'s `rulings`, the shape the
// CLI's `plan --json` prints and `test/fixtures/plan.json` holds). The Android
// twin of AgentKit's `PlanRulings.swift`. EXPERIMENTAL with the plan layer,
// behind the runner's `board_rulings` capability.
//
// A ruling is open until the owner acts (ov-333). Keep is the owner's own mark
// and changes nothing in the plan; Reverse asks the orchestrator, which does
// the work and marks the ruling reversed with the commit; Discuss starts a
// message about it. [RulingActions] holds the rules for the last two, word for
// word AgentKit's. The store's words are standing and confirmed; the owner
// reads open and kept. Copy reference is Material's: sentence case.

/** Where a ruling stands. */
enum class RulingState(val wire: String) {
    STANDING("standing"),
    CONFIRMED("confirmed"),
    REVERSED("reversed"),
    UNKNOWN("unknown");

    companion object {
        fun parse(word: String?): RulingState = entries.firstOrNull { it.wire == word } ?: UNKNOWN
    }
}

/** One ruling, as the plan read it. */
data class PlanRuling(
    val id: String,
    /** "R-12": how the owner names it to the orchestrator. */
    val short: String,
    val number: Int,
    /** What was decided, in one line. */
    val decision: String,
    /** Why, in one or two lines. */
    val why: String,
    /** What reversing it costs. */
    val reversal: String,
    val cards: List<PlanCardRef> = emptyList(),
    /** The theme's name; empty when it names none. */
    val theme: String = "",
    val state: RulingState = RulingState.STANDING,
    /** The words given when it was confirmed or reversed. */
    val note: String = "",
    val actor: String = "",
    val createdAt: Long = 0,
    val settledAt: Long? = null,
    /** Who kept or reversed it ("user" for the owner); null while it's open. */
    val settledBy: String? = null,
    /** The commit that reversed it, when the orchestrator marked it with one. */
    val reversedSha: String? = null,
    /** The theme it names, by id; null when it names none (ov-331: a theme's last move counts its rulings). */
    val themeId: String? = null,
) {
    /** What Copy reference puts on the clipboard, for telling the orchestrator. */
    val reference: String get() = "ruling $short: $decision"

    /** Open until the owner keeps or reverses it. */
    val isStanding: Boolean get() = state == RulingState.STANDING

    companion object {
        /** A plan answer's `rulings`; none when absent, as from an older runner. */
        fun decodeAll(plan: JsonObject): List<PlanRuling> =
            (plan["rulings"] as? JsonArray ?: JsonArray(emptyList())).map { decode(it.jsonObject) }

        private fun JsonObject.text(name: String): String = this[name]?.jsonPrimitive?.contentOrNull ?: ""

        fun decode(o: JsonObject): PlanRuling {
            val number = o["number"]?.jsonPrimitive?.intOrNull ?: 0
            return PlanRuling(
                id = o.text("id"),
                short = o.text("short").ifEmpty { "R-$number" },
                number = number,
                decision = o.text("decision"),
                why = o.text("why"),
                reversal = o.text("reversal"),
                cards = (o["cards"] as? JsonArray ?: JsonArray(emptyList())).map {
                    val c = it.jsonObject
                    PlanCardRef(c.text("task"), c.text("key"))
                },
                theme = o.text("theme"),
                state = RulingState.parse(o["state"]?.jsonPrimitive?.contentOrNull),
                note = o.text("note"),
                actor = o.text("actor"),
                createdAt = o["created_at"]?.jsonPrimitive?.longOrNull ?: 0L,
                settledAt = o["settled_at"]?.jsonPrimitive?.longOrNull,
                settledBy = o["settled_by"]?.jsonPrimitive?.contentOrNull,
                reversedSha = o["reversed_sha"]?.jsonPrimitive?.contentOrNull,
                themeId = o["theme_id"]?.jsonPrimitive?.contentOrNull,
            )
        }
    }
}

/** The open rulings, newest first: all that Decided for you shows. */
val Plan.openRulings: List<PlanRuling> get() = rulings.filter { it.isStanding }

/** The kept and reversed ones, most recently settled first: Past decisions. */
val Plan.pastRulings: List<PlanRuling>
    get() = rulings.filterNot { it.isStanding }
        .sortedWith(compareByDescending<PlanRuling> { it.settledAt ?: 0L }.thenByDescending { it.number })

/** What the Decided for you section says, in Material's sentence case. */
object RulingWords {
    const val TITLE = "Decided for you"
    const val COPY_REFERENCE = "Copy reference"
    const val COPIED = "Reference copied"
    const val KEEP = "Keep"
    const val KEEP_ALL = "Keep all"
    const val REVERSE = "Reverse"
    const val DISCUSS = "Discuss"
    /** The fold holding the kept and reversed rulings. */
    const val PAST_DECISIONS = "Past decisions"
    /** Why Reverse and Discuss are off, and what turns them on. */
    const val NEEDS_ORCHESTRATOR = "Start an orchestrator to reverse or discuss a ruling"
    const val WHY = "Why"
    const val REVERSAL = "Reversing"

    /** A ruling's state in the owner's words: open, kept, reversed. */
    fun state(state: RulingState): String = when (state) {
        RulingState.STANDING -> "Open"
        RulingState.CONFIRMED -> "Kept"
        RulingState.REVERSED -> "Reversed"
        RulingState.UNKNOWN -> "Unknown"
    }

    /** "Kept", "Reversed in 6e7e5618": a past decision's state, with the commit that reversed it when named. */
    fun settled(r: PlanRuling): String {
        val word = state(r.state)
        val sha = r.reversedSha
        return if (r.state == RulingState.REVERSED && !sha.isNullOrEmpty()) "$word in $sha" else word
    }

    /** "ov-1, ov-2 · Visual language", or null when it touches nothing. */
    fun touches(r: PlanRuling): String? {
        val parts = mutableListOf<String>()
        if (r.cards.isNotEmpty()) parts += r.cards.joinToString(", ") { it.key }
        if (r.theme.isNotEmpty()) parts += r.theme
        return parts.takeIf { it.isNotEmpty() }?.joinToString(" · ")
    }

    /** What TalkBack reads for a ruling. */
    fun accessibility(r: PlanRuling): String {
        val parts = mutableListOf("Ruling ${r.short}", r.decision, "$WHY: ${r.why}", "$REVERSAL: ${r.reversal}")
        if (!r.isStanding) parts += state(r.state)
        return parts.joinToString(". ")
    }
}
