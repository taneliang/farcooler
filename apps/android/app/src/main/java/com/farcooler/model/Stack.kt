package com.farcooler.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

// Where a branch sits, and what GitHub says about it.
//
// The last of the family the parity inventory found "routed in Rust and never
// called from Kotlin": `stack.get` and `pr.refresh` have been in the protocol
// since the review surface landed and no phone had reached either. Neither
// needs a big screen — a stack is a short chain and a PR's state is four words
// — so neither had a reason to stay on the Mac.
//
// `stack_json` in `crates/client/src/session.rs` is the producer, and this is
// that JSON key for key. Both calls answer with the SAME shape: `pr.refresh`
// re-reads GitHub and hands back a fresh `StackLinkList`, so the refresh
// affordance replaces the reply it already has rather than merging into it.
//
// Everything defaults, for the reason `ChangeCommit` gives at length: one
// absent key on an older runner must not fail the decode of the whole chain.

/**
 * One branch's place in a stack.
 *
 * `ahead` and `behind` are against [parentBranch], not against the worktree's
 * review base — a stack link is about branches, and the two are the same thing
 * only when the base happens to be the parent.
 */
@Serializable
data class StackLink(
    val branch: String = "",
    @SerialName("parentBranch") val parentBranch: String = "",
    /**
     * Only a GUESS is worth labeling, and `session.rs` decides that on this
     * side of the wire: `ParentSource` has four values and this is true for
     * exactly one of them. The others are recorded facts; a guessed parent
     * produces a wrong diff that looks exactly like a right one, which is the
     * same hazard `ChangeSet.baseIsGuessed` carries and is marked the same way.
     */
    @SerialName("parentGuessed") val parentGuessed: Boolean = false,
    val ahead: Int = 0,
    val behind: Int = 0,
    val pr: PullRequest? = null,
)

/**
 * What GitHub last said about the pull request on a branch.
 *
 * [state], [checks] and [review] are wire words, lowercased and fixed:
 * `session.rs` maps each enum onto a closed set and sends `"unknown"` for
 * anything it does not recognize, so this side never sees a raw number and
 * never has to invent a word for one.
 */
@Serializable
data class PullRequest(
    val number: Int = 0,
    val url: String = "",
    /** `open` · `draft` · `merged` · `closed` · `unknown`. */
    val state: String = "unknown",
    /** `passing` · `failing` · `pending` · `unknown`. */
    val checks: String = "unknown",
    /** `approved` · `changes_requested` · `review_required` · `unknown`. */
    val review: String = "unknown",
    /**
     * Read from GitHub long enough ago to doubt.
     *
     * Shown rather than hidden, and that is the whole reason Refresh exists
     * beside it: a cached "passing" is the one reading that would mislead, and
     * it is indistinguishable from a fresh one without this.
     */
    val stale: Boolean = false,
)

/** What `stack.get` and `pr.refresh` both answer with. */
@Serializable
data class StackReply(
    /**
     * The parent chain formed a loop.
     *
     * Reported rather than followed. The daemon walks as far as it walked and
     * says so; a client that drew the result as a clean stack would be drawing
     * one that does not exist.
     */
    @SerialName("cycleDetected") val cycleDetected: Boolean = false,
    val links: List<StackLink> = emptyList(),
)

// ---- the words, where a test can read them ----
//
// Pure and out here rather than built inside a composable, for the reason
// `ChangesSheets` gives at the same place: there is no emulator for this phase,
// so a sentence built inside a composable is a sentence nothing can check.

/**
 * How far this branch has drifted from its parent, as one phrase.
 *
 * Both numbers or neither. "Up to date" is a real answer and a shorter one than
 * `0 ahead · 0 behind`, which is four words spent to say nothing happened.
 */
fun StackLink.driftSentence(): String = when {
    ahead == 0 && behind == 0 -> "Up to date with ${parentBranch.ifBlank { "its parent" }}"
    behind == 0 -> "$ahead ahead"
    ahead == 0 -> "$behind behind"
    else -> "$ahead ahead · $behind behind"
}

/**
 * A pull request's state, in this app's own words rather than the wire's.
 *
 * `review_required` is the one that cannot be shown raw — an underscore in the
 * middle of a sentence is a leaked wire value, which is the thing the Apple
 * copy conventions rule out. The rest are single words and only need a capital.
 */
fun prStateWord(state: String): String = when (state) {
    "open" -> "Open"
    "draft" -> "Draft"
    "merged" -> "Merged"
    "closed" -> "Closed"
    else -> "Unknown"
}

/** See [prStateWord]. */
fun prChecksWord(checks: String): String = when (checks) {
    "passing" -> "Passing"
    "failing" -> "Failing"
    "pending" -> "Pending"
    else -> "Unknown"
}

/**
 * See [prStateWord]. Null where GitHub has no decision to report, so a row can
 * be left off entirely rather than reading "Review: Unknown" on every PR
 * nobody has looked at — which is most of them, most of the time.
 */
fun prReviewWord(review: String): String? = when (review) {
    "approved" -> "Approved"
    "changes_requested" -> "Changes requested"
    "review_required" -> "Review required"
    else -> null
}
