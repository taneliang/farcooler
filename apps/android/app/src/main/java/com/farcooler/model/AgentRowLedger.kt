package com.farcooler.model

/**
 * What one page or follow changed, so the store publishes only when something
 * did: AgentKit's `AgentRowDelta`.
 */
data class AgentRowDelta(
    /** Every held row's id, oldest first, when a row arrived, went or moved; null when only contents changed. */
    val order: List<String>? = null,
    /** Rows that arrived or whose contents changed. */
    val rows: List<AgentRow> = emptyList(),
    /** Rows no longer held. */
    val removed: List<String> = emptyList(),
    val epoch: Long = 0,
    val rev: Long = 0,
    val moreBefore: Boolean = false,
) {
    val isEmpty: Boolean get() = order == null && rows.isEmpty() && removed.isEmpty()
}

/** What a follow answered, applied. */
sealed interface AgentRowFollowed {
    data class Changed(val delta: AgentRowDelta) : AgentRowFollowed

    /** The runner can't say what changed since our cursor: page again. */
    data object Reset : AgentRowFollowed
}

/**
 * A terminal's rows as the client holds them: AgentKit's `AgentRowLedger`.
 *
 * Applies pages and follow diffs by id, keeping a row that did not change as
 * the same object, so a list keyed by id redraws only the rows that changed.
 * Synchronized, because the follow loop and an older page both reach it.
 */
class AgentRowLedger {
    private var rows = HashMap<String, AgentRow>()
    private var order = ArrayList<String>()
    private var epoch = 0L
    private var rev = 0L
    /** Whether older rows exist than the oldest held. */
    @get:Synchronized
    var moreBefore = false
        private set

    /** The cursor a follow continues from, or null before anything was held. */
    @get:Synchronized
    val cursor: Pair<Long, Long>?
        get() = if (epoch == 0L && order.isEmpty()) null else epoch to rev

    /** The oldest held row's `ord`, which an older page is asked `before`. */
    @get:Synchronized
    val oldestOrd: Long?
        get() = order.firstOrNull()?.let { rows[it]?.ord }

    /** The held rows, oldest first. */
    @Synchronized
    fun held(): List<AgentRow> = order.mapNotNull { rows[it] }

    /**
     * `agent.rows`'s answer, the newest rows: replaces what is held unless it
     * is the same projection, in which case rows older than the page stay.
     */
    @Synchronized
    fun replace(page: AgentRowPage): AgentRowDelta {
        val samePlace = page.epoch == epoch && order.isNotEmpty()
        val floor = page.rows.firstOrNull()?.ord ?: Long.MAX_VALUE
        // Rows below the page are kept when the projection is the same one:
        // `ord` never moves, so they are still where they were.
        val kept = if (samePlace) order.filter { (rows[it]?.ord ?: 0L) < floor } else emptyList()
        val next = HashMap<String, AgentRow>(kept.size + page.rows.size)
        for (id in kept) rows[id]?.let { next[id] = it }
        val changed = ArrayList<AgentRow>()
        for (row in page.rows) {
            val held = rows[row.id]
            // The held object stays when nothing changed: its row doesn't redraw.
            next[row.id] = if (held == row) held else row
            if (held != row) changed.add(row)
        }
        val nextOrder = ArrayList<String>(kept.size + page.rows.size)
        nextOrder.addAll(kept)
        page.rows.mapTo(nextOrder) { it.id }
        val removed = order.filter { it !in next }
        rows = next
        order = nextOrder
        epoch = page.epoch
        rev = page.rev
        moreBefore = if (samePlace && kept.isNotEmpty()) moreBefore else page.moreBefore
        return stamped(AgentRowDelta(order = nextOrder.toList(), rows = changed, removed = removed))
    }

    /**
     * An older page (`agent.rows {before: oldestOrd}`), put above what is held.
     * A page from another projection is dropped: the follow will reset.
     */
    @Synchronized
    fun older(page: AgentRowPage): AgentRowDelta {
        if (page.epoch != epoch) return stamped(AgentRowDelta())
        val floor = oldestOrd ?: Long.MAX_VALUE
        val fresh = page.rows.filter { it.ord < floor && it.id !in rows }
        moreBefore = page.moreBefore
        if (fresh.isEmpty()) return stamped(AgentRowDelta())
        for (row in fresh) rows[row.id] = row
        order = ArrayList(fresh.map { it.id } + order)
        return stamped(AgentRowDelta(order = order.toList(), rows = fresh))
    }

    /** `agent.rows_follow`'s answer, applied by id. */
    @Synchronized
    fun apply(changes: AgentRowChanges): AgentRowFollowed {
        if (changes.reset || changes.epoch != epoch) return AgentRowFollowed.Reset
        val changed = ArrayList<AgentRow>()
        val removed = ArrayList<String>()
        val appended = ArrayList<AgentRow>()
        var reorder = false
        val newest = order.lastOrNull()?.let { rows[it]?.ord }
        for (change in changes.changes) {
            when (change) {
                is AgentRowChanges.Change.Insert -> upsert(change.row, newest, changed, appended)
                is AgentRowChanges.Change.Update -> upsert(change.row, newest, changed, appended)
                is AgentRowChanges.Change.Remove -> {
                    if (rows.remove(change.id) == null) continue
                    removed.add(change.id)
                    reorder = true
                }
            }
        }
        var nextOrder: List<String>? = null
        if (appended.isNotEmpty() || reorder) {
            if (reorder) {
                val gone = removed.toSet()
                order.removeAll { it in gone }
            }
            appended.sortBy { it.ord }
            appended.mapTo(order) { it.id }
            nextOrder = order.toList()
        }
        rev = changes.rev
        return AgentRowFollowed.Changed(stamped(AgentRowDelta(order = nextOrder, rows = changed, removed = removed)))
    }

    private fun upsert(row: AgentRow, newest: Long?, changed: MutableList<AgentRow>, appended: MutableList<AgentRow>) {
        val held = rows[row.id]
        if (held != null) {
            if (held == row) return
            rows[row.id] = row
            changed.add(row)
        } else if (newest == null || row.ord > newest) {
            // New below everything held. A row older than the window is
            // someone else's page and isn't drawn yet.
            rows[row.id] = row
            appended.add(row)
            changed.add(row)
        }
    }

    private fun stamped(delta: AgentRowDelta) = delta.copy(epoch = epoch, rev = rev, moreBefore = moreBefore)
}
