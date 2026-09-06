package com.farcooler.model

/**
 * The agent transcript's `LazyColumn` item list, and the rule that decides
 * whether it is still following its own tail.
 *
 * ## Why the model owns the item list
 *
 * [ChangesState.Jump] already makes this argument for the diff surface and it
 * applies here word for word: Compose has no `scrollTo(id)`, `LazyListState`
 * moves by INDEX, and an index into a lazy list is only knowable from the list
 * itself. `AgentScreen` used to build its items inline and then scroll to
 * `transcript.rows.size`, which put the layout in two places — the item list
 * and whatever the scroll code believed it to be — and the two disagreed.
 *
 * The item list is a banner that is there only while a poll is failing, then
 * the rows, then a working row that is there only mid-turn. So the last index
 * is `(banner) + rows.size - 1 + (working)`, and `rows.size` is that number
 * only when EXACTLY ONE of the two optional items is present. With neither —
 * the idle case, which is most of the time — it was one past the end. With both
 * it was one short.
 *
 * That the app did not look obviously broken is what let it survive.
 * `animateScrollToItem` has no upper-bound precondition (the only `require` in
 * `foundation-android` is "Index should be non-negative"), the measure pass
 * clamps a first-visible index into range, and the animation stops when the
 * list can no longer consume the delta — so an index past the end lands at the
 * true bottom, which is the intent, by accident. The case that bites is the one
 * where the number names a REAL row: `animateScrollToItem` aligns that item's
 * TOP with the top of the viewport, so a final message taller than the screen
 * left the streaming text below the fold with the reader looking at its
 * opening paragraph.
 *
 * [TranscriptItem.End] is why that class of bug is now gone rather than fixed
 * once. It is a one-pixel item at the bottom, it is always present, so the
 * target is `items.lastIndex` and there is no arithmetic left to get wrong;
 * and because it cannot be brought to the top of a viewport, scrolling to it is
 * scrolling to the end. It is the same device the Mac uses — `AgentSurface`'s
 * `endOfTranscript` sentinel, targeted with `anchor: .bottom`.
 */
sealed interface TranscriptItem {
    /** What the `LazyColumn` keys this item on. */
    val key: String

    /**
     * The banner that says the last poll did not land.
     *
     * Its content stays in the view — it is a sentence and a detail box, which
     * is drawing — and only its PRESENCE is layout, which is what an index
     * depends on.
     */
    data object Notice : TranscriptItem {
        override val key: String get() = "notice"
    }

    /** One row of the conversation. */
    data class Row(val row: TranscriptRow) : TranscriptItem {
        override val key: String get() = "row-${row.id}"
    }

    /** The turn that is still running, one line ahead of what it has produced. */
    data object Working : TranscriptItem {
        override val key: String get() = "working"
    }

    /** The end of the content, and what following the tail targets. */
    data object End : TranscriptItem {
        override val key: String get() = "end"
    }
}

/**
 * The items the transcript draws, in the order it draws them.
 *
 * A pure function of the three things that decide the layout, so the view can
 * build its `LazyColumn` and aim its scroll from ONE value in ONE
 * recomposition. [TranscriptLayoutTest] pins the last index for all four
 * combinations of the two optional items, which is the check the old inline
 * arithmetic had no way to have.
 */
fun transcriptItems(
    hasNotice: Boolean,
    rows: List<TranscriptRow>,
    isWorking: Boolean,
): List<TranscriptItem> {
    val out = ArrayList<TranscriptItem>(rows.size + 3)
    if (hasNotice) out.add(TranscriptItem.Notice)
    for (row in rows) out.add(TranscriptItem.Row(row))
    if (isWorking) out.add(TranscriptItem.Working)
    out.add(TranscriptItem.End)
    return out
}

/**
 * Where the list is, as the three numbers a `LazyListState` reports.
 *
 * [atEnd] is `!canScrollForward`: there is nothing below the fold. Compose
 * answers that exactly, so this needs none of the slack the Mac's version
 * carries for a scroll offset that is rarely exact after a redraw.
 */
data class TailSample(
    val firstIndex: Int,
    val firstOffset: Int,
    val atEnd: Boolean,
)

/**
 * Whether the transcript should keep scrolling itself to the bottom.
 *
 * ## Detaching asks what the READER did, not how far the end is
 *
 * This is `AgentSurface.swift`'s rule, ported. The distance to the end is the
 * wrong question while a reply streams, and the version this replaces asked it
 * in items rather than in points: `last >= totalItemsCount - 2`, where `last`
 * was the last visible index. Both halves of what the Mac documents about that
 * question reproduce in Compose.
 *
 * It detached with nobody having touched the scroll. `AgentStream.pump` applies
 * a whole batch and bumps the revision ONCE, in its `finally`, and the poll is
 * 700 ms apart — so one revision routinely lands several rows at a time: a
 * message, a tool call, its result. Two new items below the fold drop the last
 * visible index to `total - 3`, the predicate went false, and because following
 * was then off the list never returned to the tail and the predicate never
 * recovered. Following was latched off for the rest of the turn.
 *
 * And it failed to detach when the reader HAD scrolled. Two items of slack is
 * two items, not a distance, and a tool-call row and the working row are one
 * line each — so a reader who dragged up past two short rows was still "at the
 * tail" by that predicate and got animated back down on the next poll.
 *
 * ## What replaces it
 *
 * Growth moves the content; only a reader moves the list BACKWARDS, because the
 * screen's own scroll only ever moves it toward the end. So detaching asks
 * whether the first visible position went backwards, and re-attaching asks the
 * original "is the end on screen" question.
 *
 * Re-attaching is tested FIRST, for the reason the Mac gives: a row whose
 * height resolves shorter than it was measured, or the working row going away
 * at the end of a turn, walks the position back while the end stays on screen,
 * and that must not read as scrolling up. It is not a complete defense — a
 * transcript replaced wholesale by [Transcript.resetForNewEpoch] can move the
 * position back while the end is off screen — and the Mac carries the same
 * residual risk. Following is restored there the moment the reader reaches the
 * bottom, which is the same gesture they would already be making.
 *
 * ## What is deliberately NOT ported
 *
 * The Mac stopped ANIMATING this scroll, and that reasoning does not carry.
 * `Motion.snap` is a 0.22 s spring against a 0.2 s poll, so a Mac's scroll
 * could never settle and the scroll view was in motion for the whole of a turn
 * by construction. `AgentStream.POLL_INTERVAL_MS` is 700 ms, chosen to be
 * slower than the Mac's on purpose, so an `animateScrollToItem` finishes
 * between revisions. A finger is safe from it besides: a drag takes
 * `MutatePriority.UserInput` and `LazyListState.animateScrollToItem` takes
 * `MutatePriority.Default`, so the gesture preempts the animation rather than
 * fighting it.
 */
object TranscriptTail {
    fun following(was: Boolean, previous: TailSample, now: TailSample): Boolean = when {
        now.atEnd -> true
        now.firstIndex < previous.firstIndex -> false
        now.firstIndex == previous.firstIndex && now.firstOffset < previous.firstOffset -> false
        else -> was
    }
}
