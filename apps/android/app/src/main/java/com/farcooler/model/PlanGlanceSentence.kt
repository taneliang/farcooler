package com.farcooler.model

// The Plan's glance band (ov-503, design 2): one line for Now and one for Next. `PlanStrip` carries it, so the folded
// strip, the phones and the watch's glance read the same sentence. The Android twin of AgentKit's
// `PlanGlanceSentence.swift`.
//
//   Now   Train 86 waiting for CI · 2 building · 3 in review
//   Next  reps — Each person's asks reach the orchestrator, then skill-init
//
// The Now line names the furthest-along train first, because shipping is what moves main, then counts what's building
// and in review. The Next line names the first ranked lane and the second.

data class PlanGlanceSentence(
    /** "Train 86 waiting for CI · 2 building · 3 in review"; null when nothing is shipping, building or in review. */
    val now: String? = null,
    /** "reps — Each person's asks…, then skill-init"; null when nothing is ranked. */
    val next: String? = null,
) {
    /** The Now line as the band says it. */
    val nowLine: String get() = now ?: NOTHING_RUNNING

    /** The Next line as the band says it. */
    val nextLine: String get() = next ?: NOTHING_QUEUED

    /** Both as one sentence: "Nothing running. Next: nothing queued." */
    val text: String get() = "$nowLine. Next: ${next ?: NOTHING_QUEUED.lowercase()}."

    companion object {
        const val NOTHING_RUNNING = "Nothing running"
        const val NOTHING_QUEUED = "Nothing queued"

        fun of(sections: PlanSections): PlanGlanceSentence {
            val parts = mutableListOf<String>()
            sections.shipping.firstOrNull()?.let { lead ->
                parts += "${lead.train.heading} ${lead.phase.sentence}"
                val more = sections.shipping.size - 1
                if (more > 0) parts += if (more == 1) "1 more train" else "$more more trains"
            }
            if (sections.building.isNotEmpty()) parts += "${sections.building.size} building"
            if (sections.inReview.isNotEmpty()) parts += "${sections.inReview.size} in review"

            val ranked = sections.waiting.next.map { it.lane }
            val next = ranked.firstOrNull()?.let { first ->
                // The lane's title after its name, when it has one.
                var said = if (first.slug == null) first.name else "${first.name} — ${first.heading}"
                if (ranked.size > 1) said += ", then ${ranked[1].name}"
                said
            }
            return PlanGlanceSentence(now = if (parts.isEmpty()) null else parts.joinToString(" · "), next = next)
        }
    }
}
