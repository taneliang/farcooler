package com.farcooler.model

/**
 * Where a dragged workspace card lands, worked out without Compose.
 *
 * No Compose type appears in this file, which is the same rule `Shell.kt`
 * follows and for the same reason: this app has no Compose UI test dependency,
 * so anything that lives inside a `@Composable` can only ever be checked by a
 * person dragging at it on a phone. Drag-and-drop is the worst possible thing to
 * check that way — the finger is moving, the list is re-laying out, and a drop
 * that lands one card short looks exactly like a drop that lands right.
 *
 * The Swift half of this is `apps/shared/AgentKit/Sources/AgentKit/WorkspaceOrder.swift`
 * and the two must agree: they drag the same cards, into the same order, on the
 * same runner. `moved` and `edge` here are that file line for line, and the test
 * cases are the same cases.
 *
 * It knows nothing about activity, attention or recency, and it must never
 * learn: the whole point of a stored order is that a card stays where it was put.
 */
object WorkspaceOrder {

    /** Which side of the card under the finger a drop would go. */
    enum class Edge { ABOVE, BELOW }

    /** A card's vertical extent in the list, in pixels. */
    data class Card(val id: String, val top: Int, val bottom: Int)

    /** Where a finished drag would insert. */
    data class Landing(val target: String, val edge: Edge)

    /**
     * The half of a card the finger is in.
     *
     * A midpoint rather than a margin at each end. A margin leaves a dead band
     * through the middle of every card where a drop means nothing, and what a
     * person does with a card is aim at the gap between two others — so the
     * gesture would fail most often exactly where it is aimed most carefully.
     *
     * A card with no height reads as [Edge.ABOVE], which is the answer that
     * cannot move something past somewhere it has not been dragged.
     */
    fun edge(pointerY: Float, rowHeight: Float): Edge =
        if (rowHeight > 0f && pointerY >= rowHeight / 2f) Edge.BELOW else Edge.ABOVE

    /** One item a lazy list has laid out: its key, and where it sits. */
    data class Laid(val id: String, val offset: Int, val size: Int)

    /**
     * Card extents from what a lazy list has actually laid out.
     *
     * A workspace in this list is not one item. It is a header followed by
     * however many terminal rows, all siblings in one flat `LazyColumn` — so a
     * card runs from its own header to the NEXT header, and the terminals in
     * between belong to the header above them. Measuring the headers alone
     * would leave every gap between them dead, which is most of the screen.
     *
     * [laid] is every visible item in list order; [headers] names the ones that
     * start a card. Items before the first header and after the last are
     * ignored except that the last one sets where the final card ends.
     */
    fun cards(laid: List<Laid>, headers: Set<String>): List<Card> {
        if (laid.isEmpty()) return emptyList()
        val tops = laid.filter { it.id in headers }
        val end = laid.last().let { it.offset + it.size }
        return tops.mapIndexed { i, item ->
            Card(item.id, item.offset, if (i + 1 < tops.size) tops[i + 1].offset else end)
        }
    }

    /**
     * Which card the finger is over, and which half of it.
     *
     * [cards] are the workspace cards currently laid out, in list order, each
     * running from its own top to the next one's — so the terminals under a
     * header belong to that header, and a finger anywhere in a workspace's block
     * is over that workspace.
     *
     * Clamped at both ends rather than answering null: dragging above the first
     * card means the front of the list and dragging below the last means the
     * end, and those are the two moves a person makes most. Null is only for an
     * empty list.
     */
    fun landing(cards: List<Card>, pointerY: Int): Landing? {
        if (cards.isEmpty()) return null
        val first = cards.first()
        if (pointerY < first.top) return Landing(first.id, Edge.ABOVE)
        val last = cards.last()
        if (pointerY >= last.bottom) return Landing(last.id, Edge.BELOW)
        val over = cards.lastOrNull { pointerY >= it.top } ?: first
        return Landing(over.id, edge((pointerY - over.top).toFloat(), (over.bottom - over.top).toFloat()))
    }

    /**
     * [ids] with [dragged] lifted out and put back against [target].
     *
     * Returns the list UNCHANGED when the drop would not move anything: the card
     * dropped on itself, either id missing from the list, or the card already
     * exactly where the drop would put it. Callers lean on that — comparing the
     * answer to what they started with is how they decide whether to spend a
     * round trip, and a reorder request that changes nothing still makes every
     * other client re-read the fleet.
     *
     * The target's index is taken AFTER the dragged card is removed, which is
     * what makes dragging downwards land where the finger is rather than one
     * short of it — the classic off-by-one in every hand-written reorder.
     */
    fun moved(ids: List<String>, dragged: String, target: String, edge: Edge): List<String> {
        if (dragged == target) return ids
        val from = ids.indexOf(dragged)
        if (from < 0 || !ids.contains(target)) return ids

        val next = ids.toMutableList()
        next.removeAt(from)
        val landing = next.indexOf(target)
        if (landing < 0) return ids
        next.add(if (edge == Edge.ABOVE) landing else landing + 1, dragged)
        return if (next == ids) ids else next
    }
}
