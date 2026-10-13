package com.farcooler.model

// The themes grid (ov-503, design 5): a tile a theme, modeled on the site's "What it's building now". The Android twin
// of AgentKit's `PlanThemeTile.swift`: which cards, which themes lead and which are finished are values here.

/** A card's mark on a tile and in the sidebar: the site's three. */
enum class PlanCardGlyph {
    DONE, IN_PROGRESS, NOT_STARTED;

    companion object {
        /** In review counts as in progress; waiting on a decision hasn't started, and the tile's ask line says who it waits on. */
        fun of(status: TaskStatus): PlanCardGlyph = when (status) {
            TaskStatus.DONE -> DONE
            TaskStatus.IN_PROGRESS, TaskStatus.IN_REVIEW -> IN_PROGRESS
            TaskStatus.BACKLOG, TaskStatus.TODO, TaskStatus.NEEDS_DECISION, TaskStatus.CANCELLED -> NOT_STARTED
        }
    }
}

data class PlanThemeTile(
    val id: String,
    val name: String,
    val done: Int,
    /** The cards that count: all but the canceled. */
    val total: Int,
    /** Up to three, in progress first, then the most recent done, then the next not started. */
    val cards: List<Card>,
    /** What waits on the owner: the one amber line. Null when nothing does. */
    val ownerAsk: String?,
    /** A lane of it is shipping, in review or building. */
    val isMoving: Boolean,
    /** When it last moved, in ms. */
    val lastMoved: Long,
    /** Every card done, or the theme marked so. */
    val isFinished: Boolean,
) {
    data class Card(val task: String, val key: String, val title: String, val status: TaskStatus) {
        val glyph: PlanCardGlyph get() = PlanCardGlyph.of(status)
    }

    /** "6 of 10 done". */
    val progress: String get() = "$done of $total done"

    /** How full the meter is, 0 to 1. */
    val fraction: Double get() = if (total > 0) done.toDouble() / total else 0.0

    companion object {
        /** How many cards a tile shows. */
        const val CARDS_SHOWN = 3

        fun of(theme: PlanTheme, plan: Plan, moving: Set<String>, statuses: Map<String, TaskStatus>, activity: Map<String, Long>): PlanThemeTile {
            val total = PlanWords.total(theme.counts)
            // The board's status wins; else the plan's own word for it. A card the plan can't place, or one that was
            // canceled, isn't shown.
            val listed = theme.cards.mapNotNull { ref ->
                val known = plan.card(ref.task)
                val status = statuses[ref.task] ?: known?.let { OneTree.status(it.status) }
                if (status == null || status == TaskStatus.CANCELLED) null
                else Card(ref.task, ref.key, known?.title ?: "", status)
            }
            val notStarted = listed.firstOrNull { it.glyph == PlanCardGlyph.NOT_STARTED }?.let { listOf(it) } ?: emptyList()
            val inProgress = listed.filter { it.glyph == PlanCardGlyph.IN_PROGRESS }.take(CARDS_SHOWN - notStarted.size)
            val room = CARDS_SHOWN - notStarted.size - inProgress.size
            // Later in a theme's list is later added; the board's word on when it moved breaks that.
            val finished = listed.withIndex().filter { it.value.glyph == PlanCardGlyph.DONE }
                .sortedWith(compareByDescending<IndexedValue<Card>> { activity[it.value.task] ?: 0L }.thenByDescending { it.index })
                .take(room).map { it.value }
            val tasks = theme.cards.map { it.task }
            return PlanThemeTile(
                id = theme.id, name = theme.name, done = theme.counts.done, total = total,
                cards = inProgress + finished + notStarted, ownerAsk = theme.ownerAsk.ifEmpty { null },
                isMoving = plan.lanesIn(theme).any { it.id in moving },
                lastMoved = maxOf(plan.lastMoved(theme), tasks.maxOfOrNull { activity[it] ?: 0L } ?: 0L),
                isFinished = theme.state == "done" || (total > 0 && theme.counts.done == total),
            )
        }
    }
}

/** The tiles, and the one tile for what's finished. */
data class PlanThemeGrid(
    /** Themes in motion first, then the rest by last move. */
    val tiles: List<PlanThemeTile>,
    /** Done themes, which the grid folds into "Finished". */
    val finished: List<PlanThemeTile>,
) {
    val isEmpty: Boolean get() = tiles.isEmpty() && finished.isEmpty()

    /** "Finished", the one tile's title; null when nothing is. */
    val finishedTitle: String? get() = if (finished.isEmpty()) null else "Finished"

    /** "7 themes", "1 theme". */
    val finishedWords: String? get() = if (finished.isEmpty()) null else if (finished.size == 1) "1 theme" else "${finished.size} themes"

    companion object {
        fun of(plan: Plan, moving: Set<String>, statuses: Map<String, TaskStatus> = emptyMap(), activity: Map<String, Long> = emptyMap()): PlanThemeGrid {
            val all = plan.themes.filter { it.state != "dropped" }.map { PlanThemeTile.of(it, plan, moving, statuses, activity) }
            val ordinal = plan.themes.associate { it.id to it.ordinal }
            val tiles = all.filter { !it.isFinished }.sortedWith(
                compareByDescending<PlanThemeTile> { it.isMoving }.thenByDescending { it.lastMoved }.thenBy { ordinal[it.id] ?: 0L },
            )
            return PlanThemeGrid(tiles, all.filter { it.isFinished }.sortedByDescending { it.lastMoved })
        }
    }
}
