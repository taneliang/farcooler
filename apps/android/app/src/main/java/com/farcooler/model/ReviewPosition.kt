package com.farcooler.model

import com.farcooler.data.ReviewStorage
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

// Where you were, and how this app works out where that is.
//
// A port of the half of `apps/ios/FarCooler/ChangesReview.swift` that stayed on
// the phone. The comment queue moved into AgentKit when the Mac grew the same
// feature; `ReviewPosition` did not, and the reason it did not is the reason
// this file exists on Android too — a Mac session is not terminated between
// ninety-second windows, and a bookmark is about a process that is.
//
// Nothing in this file records a JUDGMENT. There is no "reviewed" flag and no
// per-file checkmark, and that is a decision rather than an omission: an agent
// is still editing these files, so a mark saying "I read this" on a file that
// has changed twice since is a lie the app would be telling on the reader's
// behalf. The workspace-level `changed_since_reviewed` watermark the daemon
// already keeps — see [InboxRow] — is the one piece of review state that
// survives an edit, because it is invalidated BY the edit. This file remembers
// position only.

/**
 * Which worktree, on which runner.
 *
 * **The host is half of the identity, and leaving it out is a real collision
 * rather than a theoretical one.** Workspace ids are minted per daemon, so two
 * runners can hold worktrees with the same id; `df87410` had to solve exactly
 * this for the front door, where [NeedsYouSection.key] is `host/workspace` and
 * never the workspace alone, and `BackstackTest` pins that the two must not be
 * conflated. iOS keys its bookmarks on the workspace alone and is right to,
 * because that app talks to one runner at a time. This one connects to every
 * runner at once — `net/FleetRepository.kt` — so a bookmark keyed on the
 * workspace would let a phone that has read a diff on the laptop resume "where
 * it was" in a different worktree on the desktop.
 *
 * Spelled the same way [NeedsYouSection.key] is, so the two never disagree
 * about what identifies a worktree on this phone.
 */
data class ReviewRef(val hostId: String, val workspaceId: String) {
    val key: String get() = "$hostId/$workspaceId"
}

/**
 * Where one worktree's review was when the app last had a chance to notice.
 *
 * Anchors rather than pixels, and that is the load-bearing choice. The obvious
 * spelling of "remember the scroll" is a scroll offset, and an offset is a
 * promise about a layout — it means "1,840 pixels down" in a list whose shape is
 * decided by which files an agent has touched since. Come back after a set
 * during which the agent added two files above the one being read and that
 * number lands somewhere else entirely, silently. A PATH does not move: it
 * resolves to the same file whatever happened above it, and when the file is
 * gone it can be SAID that it is gone, which a number cannot do.
 *
 * That argument survives the port intact even though the mechanism does not.
 * iOS hands a path to `proxy.scrollTo`; Compose has no such call and this app
 * resolves the path to an index instead — see [Jump] and [ChangesState.rows].
 * The saved record is the same either way, which is the whole point of saving
 * an anchor: it is a fact about the diff, not about a list.
 *
 * [topFile] is the answer for the case with no expanded file — arriving at the
 * list, scrolling through twenty headings, and being interrupted. The file at
 * the top of the screen is what "how far down was I" means when nothing is open,
 * and it survives the list changing shape for the same reason.
 *
 * [savedAt] exists so a bookmark can be judged stale rather than merely old.
 * Nothing expires it today; it is recorded because the offer to resume is worth
 * phrasing differently for a position from four days ago, and the field is free
 * while the record is being written anyway.
 */
@Serializable
data class ReviewPosition(
    /** [DiffScope.wire] — `branch`, `local`, or a full sha. */
    val scope: String = DiffScope.Branch.wire,
    /** The file that was open, if one was. */
    val file: String? = null,
    /** The file that was at the top of the screen, when none was open. */
    val topFile: String? = null,
    val savedAt: Long = 0,
) {
    /**
     * Whether this says anything worth offering.
     *
     * A bookmark on `branch` with nothing open and nothing scrolled to is the
     * position everybody starts at, and offering to restore it would be an
     * interruption that resolves to a no-op.
     */
    val isSomewhere: Boolean
        get() = file != null || topFile != null || scope != DiffScope.Branch.wire

    companion object {
        /**
         * The sha a saved scope names, if it names one.
         *
         * [DiffScope.wire] is the protocol's own rule — `branch` and `local` are
         * names and anything else is a sha, which is what `Session::file_diff`
         * matches on the other end — so reading it back is the same rule in
         * reverse. Written beside the rule rather than at the call site so the
         * two halves sit together.
         */
        fun sha(scope: String): String? = when (scope) {
            "", "branch", "local", "staged", "unstaged" -> null
            else -> scope
        }
    }
}

/**
 * The bookmarks, one per worktree.
 *
 * An object over a namespaced key rather than a class with flows: nothing
 * observes a bookmark. It is written when the position changes and read once,
 * when a store is created, and anything observable in between would only publish
 * changes nobody is watching.
 */
object ReviewBookmarks {
    /** Tolerant on the way in: a record read by a newer build than wrote it. */
    private val json = Json { ignoreUnknownKeys = true }

    private fun key(ref: ReviewRef) = "changes.position.${ref.key}"

    fun read(storage: ReviewStorage, ref: ReviewRef): ReviewPosition? {
        val raw = storage.read(key(ref)) ?: return null
        return runCatching { json.decodeFromString(ReviewPosition.serializer(), raw) }.getOrNull()
    }

    fun write(storage: ReviewStorage, ref: ReviewRef, position: ReviewPosition) {
        val raw = runCatching {
            json.encodeToString(ReviewPosition.serializer(), position)
        }.getOrNull() ?: return
        storage.write(key(ref), raw)
    }

    fun forget(storage: ReviewStorage, ref: ReviewRef) {
        storage.remove(key(ref))
    }
}

/**
 * Which row is at the top of the screen, from what a `LazyColumn` says it is
 * showing.
 *
 * ## The second thing phase 5 could not port
 *
 * iOS's whole resume anchor rests on `.onScrollVisibilityChange(threshold: 0.05)`
 * on each file heading, which SwiftUI calls per view as it crosses in and out of
 * the viewport. **Compose has no such modifier.** What it has is
 * `LazyListState.layoutInfo`, which is the opposite shape: not a callback per
 * item but a snapshot of every item currently laid out, with its offset and its
 * size, read whenever somebody asks.
 *
 * That turns out to be the better shape for the question actually being asked.
 * iOS keeps a `Set` of visible paths and then computes "the reading-order first
 * one that is on screen" out of it, precisely because a per-item callback cannot
 * answer that on its own — a card appearing tells you nothing about which way
 * the scroll was going. `layoutInfo.visibleItemsInfo` arrives in index order
 * already, so the topmost visible row is the first element that passes the
 * threshold, and no set has to be maintained at all.
 *
 * Pure, and takes numbers rather than a `LazyListState`, so it is provable
 * without a device — which matters here more than usual, since no device was
 * available for any of this work.
 *
 * @param threshold how much of a row must be inside the viewport to count, as a
 *   fraction of its height. `0.05` is iOS's, and it is deliberately tiny: the
 *   question is "which file am I looking at", and a heading with a sliver
 *   showing at the top of the screen is still the file whose hunks fill it.
 */
object ReviewScroll {
    /** One laid-out row, in the terms `LazyListItemInfo` reports. */
    data class VisibleRow(val key: String, val offset: Int, val size: Int)

    const val VISIBLE_THRESHOLD = 0.05f

    /**
     * The first row far enough into the viewport to count, or null.
     *
     * Rows whose key is not a file path are skipped rather than returned: the
     * summary block is at the top of every list and is not a place in the diff,
     * so returning it would make every bookmark say "the top" and the resume
     * offer would never have anything to restore.
     */
    fun topVisible(
        rows: List<VisibleRow>,
        viewportStart: Int,
        viewportEnd: Int,
        threshold: Float = VISIBLE_THRESHOLD,
        // Last, so it can be written as a trailing lambda at the call site.
        isFile: (String) -> Boolean,
    ): String? {
        for (row in rows) {
            if (!isFile(row.key)) continue
            if (row.size <= 0) continue
            val top = maxOf(row.offset, viewportStart)
            val bottom = minOf(row.offset + row.size, viewportEnd)
            val shown = bottom - top
            if (shown <= 0) continue
            if (shown.toFloat() / row.size >= threshold) return row.key
        }
        return null
    }
}
