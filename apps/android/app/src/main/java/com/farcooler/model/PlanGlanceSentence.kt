package com.farcooler.model

// The Plan's glance band (ov-503, design 2): one line for Now and one for Next. `PlanStrip` carries it, so the folded
// strip, the phones and the watch's glance read the same sentence. The Android twin of AgentKit's
// `PlanGlanceSentence.swift`.
//
//   Now   Train 86 checking on CI · 2 building · 3 in review
//   Next  Reps: each person's asks reach the orchestrator, then Onboarding: skill-init…
//
// The Now line names the furthest-along train first, because landing is what moves main, then counts what's
// building and in review. The Next line names the first ranked lane and the second, or, with none ranked, counts
// the lanes that wait: "4 waiting, none ranked" (ov-522).
//
// The train is "Train 86", never its title: a title can run to a line, and the phase after it is what the band is for
// (ov-514). The train card shows the title.

data class PlanGlanceSentence(
    /** "Train 86 checking on CI · 2 building · 3 in review"; null when nothing is landing, building or in review. */
    val now: String? = null,
    /** "Reps: each person's asks…, then Onboarding: skill-init…", or "4 waiting, none ranked"; null when nothing is queued. */
    val next: String? = null,
) {
    /** The Now line as the band says it. */
    val nowLine: String get() = now ?: NOTHING_RUNNING

    /** The Next line as the band says it. */
    val nextLine: String get() = next ?: NOTHING_QUEUED

    /** Both as one sentence: "No lanes moving. Next: nothing queued." */
    val text: String get() = "$nowLine. Next: ${next ?: NOTHING_QUEUED.lowercase()}."

    companion object {
        /** How long each lane's title is in the Next line, before it is cut at a word. */
        const val TITLE_MAX = 72

        /** A lane's title cut at a word, with no comma or colon left hanging before the ellipsis. */
        internal fun title(lane: PlanLane): String {
            val cut = PlanWords.short(lane.heading, TITLE_MAX)
            if (!cut.endsWith("\u2026")) return cut
            return cut.dropLast(1).trimEnd { it in ",;:\u2014- " } + "\u2026"
        }

        const val NOTHING_RUNNING = "No lanes moving"
        const val NOTHING_QUEUED = "Nothing queued"

        fun of(sections: PlanSections): PlanGlanceSentence {
            val parts = mutableListOf<String>()
            sections.trains.firstOrNull()?.let { lead ->
                parts += "${lead.train.label} ${lead.phase.sentence}"
                val more = sections.trains.size - 1
                if (more > 0) parts += if (more == 1) "1 more landing" else "$more more landings"
            }
            if (sections.building.isNotEmpty()) parts += "${sections.building.size} building"
            if (sections.inReview.isNotEmpty()) parts += "${sections.inReview.size} in review"

            val ranked = sections.waiting.next.map { it.lane }
            val next = ranked.firstOrNull()?.let { first ->
                // Titles only, each said once and cut at a word: the lane's name is in its title often enough, and never twice.
                var said = title(first)
                ranked.drop(1).firstOrNull { it.id != first.id && it.heading != first.heading }?.let {
                    said += ", then ${title(it)}"
                }
                said
            } ?: if (sections.waiting.count > 0) "${sections.waiting.count} waiting, none ranked" else null
            return PlanGlanceSentence(now = if (parts.isEmpty()) null else parts.joinToString(" · "), next = next)
        }
    }
}
