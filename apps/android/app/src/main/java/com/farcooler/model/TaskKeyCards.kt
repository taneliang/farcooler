package com.farcooler.model

/**
 * What a task key is, at a glance (ov-299): the card a long press on a key
 * shows. AgentKit's `TaskKeyCards.swift`, the same rules: built from the
 * boards and plans already read, so a long press is a map lookup and never a
 * call to the runner. A key no read board has, or another runner's link, has
 * no card and shows nothing.
 */
data class TaskKeyCard(
    val key: String,
    val title: String,
    val status: TaskStatus,
    /** The plan theme it's in, by name, or null. */
    val theme: String? = null,
    /** The plan lane working it: a live one, else the last to finish; or null. */
    val lane: String? = null,
    /** Its intent's first line, or its newest written note when read; may be empty. */
    val excerpt: String = "",
) {
    /** Status, theme and lane, those it has: "In progress · Invoices · rounding". */
    val details: String get() = (listOf(status.title) + listOfNotNull(theme, lane)).joinToString(" · ")

    /** What TalkBack says for the key: the key, then the title. */
    val accessibilityLabel: String get() = "$key, $title"

    companion object {
        const val EXCERPT_LIMIT = 160

        /** [text]'s first non-empty line, Markdown's leading markers dropped, cut at [EXCERPT_LIMIT]. */
        fun excerpt(text: String): String {
            val line = text.lineSequence().map { it.trim() }.firstOrNull { it.isNotEmpty() } ?: ""
            val plain = line.trimStart('#', '>', '-', '*').trim()
            if (plain.length <= EXCERPT_LIMIT) return plain
            val cut = plain.take(EXCERPT_LIMIT - 1)
            val space = cut.lastIndexOf(' ')
            return if (space >= 0 && cut.length - space < EXCERPT_LIMIT / 5) cut.take(space).trim() + "…" else cut.trim() + "…"
        }
    }
}

/** One runner's cards, by key. */
data class TaskKeyCards(val runner: String, val cards: Map<String, TaskKeyCard>) {
    fun card(key: String): TaskKeyCard? = cards[key]

    /** The card a task link names, or null: another runner's, an unknown key, or any other URL. */
    fun cardFor(url: String): TaskKeyCard? {
        val (runner, key) = TaskKeyLinks.parse(url) ?: return null
        return if (runner == this.runner) cards[key] else null
    }

    companion object {
        val EMPTY = TaskKeyCards("", emptyMap())

        /** [runner]'s cards from its boards and their plans, by workspace id; the first board in id order wins a shared key. */
        fun of(runner: String, boards: Map<String, TaskBoard>, plans: Map<String, Plan> = emptyMap()): TaskKeyCards {
            val cards = mutableMapOf<String, TaskKeyCard>()
            for (workspace in boards.keys.sorted()) {
                val plan = plans[workspace]
                val themes = mutableMapOf<String, String>()
                val lanes = mutableMapOf<String, String>()
                if (plan != null) {
                    for (theme in plan.themes.sortedBy { it.ordinal }) for (card in theme.cards) themes.putIfAbsent(card.task, theme.name)
                    val ordered = plan.lanes.sortedWith(
                        compareByDescending<PlanLane> { it.state.isLive }.thenByDescending { it.stateSince },
                    )
                    for (lane in ordered) for (card in lane.cards) lanes.putIfAbsent(card.task, lane.name)
                }
                for (row in boards[workspace]?.rows.orEmpty()) {
                    if (row.key in cards) continue
                    cards[row.key] = TaskKeyCard(
                        key = row.key, title = row.title, status = row.status, theme = themes[row.id],
                        lane = lanes[row.id], excerpt = TaskKeyCard.excerpt(row.intent),
                    )
                }
            }
            return TaskKeyCards(runner, cards)
        }
    }
}
