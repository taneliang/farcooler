package com.farcooler.model

import com.farcooler.data.ReviewStorage
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.util.UUID

// What the reader wants to tell the agent about the diff, and how it gets
// there.
//
// Ported from `apps/shared/AgentKit/Sources/AgentKit/ReviewComments.swift`,
// which is itself the phone's original queue hoisted out of `ChangesReview.swift`
// when the Mac's diff pane grew the same feature. The reasoning in each doc
// comment is that module's and is kept as it was argued: comments are ANCHORED
// so a sentence has a referent, they are BATCHED so an agent gets one turn
// rather than five, the queue PERSISTS on every write because the app holding it
// can be killed between the window a note is written in and the window it would
// be sent in, and nothing here is EVER retried automatically because the
// protocol has no delivery receipt to retry against.
//
// One thing is abstracted rather than ported: the send. Every client calls
// `terminal.agent_prompt`, but through its own transport — this one through
// `ClientCore.call`, the Mac by shelling out — so the queue takes a closure that
// hands one message to one pane and answers with a sentence when it could not.
// The mapping from a platform's own error to that sentence stays on the
// platform, which for this one is `net/Connection.kt`.
//
// **What is deliberately not here: `putInComposer`.** AgentKit carries a second
// way out of the outbox, where a batch lands in an agent's composer for the
// reader to send themselves — the Mac's answer to the missing delivery receipt,
// since a message visibly in a transcript needs no receipt. iOS never uses it
// (`ReviewAgentTarget.showsChat` exists purely to gate the Mac's button), and it
// is not ported because nothing calls it yet. It is worth recording that this
// app is the Mac's case rather than iOS's: since `e23718c` the agent panes of
// this workspace are MOUNTED beside the Changes tab, drafts and all, so a batch
// dropped into one is a chip away rather than a screen away. That is 5c's, which
// owns the outbox sheet.

/**
 * The part of the diff a comment is about.
 *
 * The whole difference between a comment and a prompt. "Handle 429 as well" sent
 * on its own is a sentence with no referent, and an agent receiving it has to
 * guess which of the eleven files it just wrote is meant; the same sentence
 * carrying `push.ts`, around lines 120-148, and the line that was on screen is an
 * instruction that can be acted on without a search.
 *
 * The quoted line comes out of the diff the daemon already sent and is capped at
 * [QUOTE_LIMIT] here — a client-side cut for the size of a prompt, on text the
 * host had already decided to show. Nothing here re-reads a file, widens a hunk,
 * or recovers anything the daemon chose to truncate or redact.
 *
 * A range of one line is the anchor a pointer can produce and a thumb cannot, and
 * it is the BETTER instruction: `firstLine == lastLine` says exactly which line,
 * where a hunk's range says which twenty-eight to look through.
 */
@Serializable
data class ReviewAnchor(
    val file: String,
    /**
     * The commit it was written against, when it was written against one. A
     * comment on the branch as a whole carries no sha, truthfully.
     */
    val commit: String? = null,
    /**
     * The hunk's line range in the new file, when the comment was written on a
     * hunk rather than on the file as a whole.
     */
    val firstLine: Int? = null,
    val lastLine: Int? = null,
    /**
     * One line of the hunk, so the agent is told WHERE in a 300-line file rather
     * than only which file.
     */
    val quote: String? = null,
) {
    /** How this reads in the app, above the composer and in the outbox. */
    val placeDescription: String
        get() {
            val first = firstLine ?: return "the whole file"
            val last = lastLine
            if (last == null || last <= first) return "line $first"
            return "lines $first-$last"
        }

    /**
     * How this reads in the message the agent is sent.
     *
     * Backticked because the receiver is an agent reading markdown, and a path in
     * prose is a path it has to guess the boundaries of.
     */
    val promptDescription: String
        get() = buildString {
            append("`$file`")
            if (commit != null) append(" (commit ${commit.take(8)})")
            if (firstLine != null) append(", around $placeDescription")
        }

    companion object {
        const val QUOTE_LIMIT = 120

        fun quoting(text: String): String? {
            val trimmed = text.trim()
            if (trimmed.isEmpty()) return null
            if (trimmed.length <= QUOTE_LIMIT) return trimmed
            return trimmed.take(QUOTE_LIMIT) + "…"
        }
    }
}

/** One thing the reader wants to say, not yet said. */
@Serializable
data class ReviewComment(
    val id: String = UUID.randomUUID().toString(),
    val anchor: ReviewAnchor,
    val text: String,
    val writtenAt: Long = 0,
)

/**
 * A batch that left the queue, kept so the app can show WHAT was handed over.
 *
 * Required rather than nice: `session/prompt` is sent with `request_no_wait` and
 * its response signals end-of-turn, not receipt, so nothing anywhere can confirm
 * that an agent received a prompt. The only honest thing this screen can offer is
 * the text it handed over and the time it did so, which is what this is. See
 * [ReviewCommentQueue.send].
 */
@Serializable
data class SentReviewBatch(
    val id: String = UUID.randomUUID().toString(),
    val text: String,
    val agentName: String,
    val sentAt: Long,
    val count: Int,
)

/**
 * An agent pane a review comment can be sent to.
 *
 * A value rather than a [Terminal], so the diff surface does not have to hold a
 * live fleet to know what it can send to — holding one would re-evaluate the
 * diff list on every three-second poll, which is the one thing a screen built for
 * scrolling a long patch should not do.
 */
data class ReviewAgentTarget(
    /** The terminal's id, which is what `terminal.agent_prompt` names a pane by. */
    val id: String,
    val name: String,
    /**
     * Whether this pane is being DRAWN as a chat right now.
     *
     * Nothing reads it yet — see the note at the top of this file about
     * `putInComposer`. Filled in anyway, because a field one caller leaves at its
     * default is a field that is wrong the first time somebody wants it.
     */
    val showsChat: Boolean = false,
)

/**
 * The panes in this worktree a review note can be handed to.
 *
 * [Terminal.isAgentPane] OR [Terminal.canSwitchPaneMode], because both are the
 * daemon's word for "an agent is in here" and only the first is about what is
 * currently DRAWN. A claude the user has flipped back to its raw terminal is
 * still an agent holding an ACP session, and `terminal.agent_prompt` reaches it;
 * excluding it would mean a review with nowhere to send to for the sole reason
 * that somebody wanted to watch the tty.
 *
 * **A `changes` pane is excluded explicitly, which is the Mac's rule and not
 * iOS's.** The Mac folds `!isChangesPane` into its own `canSwitchPaneMode`
 * because a Mac can put a diff in a pane and the daemon refuses to switch that
 * one; iOS's cannot, so it does not. This app is the Mac's case — `e23718c`
 * folds a host-side `changes` pane into the Changes tab precisely because they
 * arrive here — and handing a review note to the diff of the thing being
 * reviewed is the one target on this list that could never receive it.
 */
fun Workspace.reviewAgentTargets(): List<ReviewAgentTarget> {
    val numbering = ordinals()
    return terminals
        .filter { (it.isAgentPane || it.canSwitchPaneMode) && !it.isChangesPane }
        .map {
            ReviewAgentTarget(
                id = it.id,
                name = it.displayName(numbering[it.id]),
                showsChat = it.isAgentPane,
            )
        }
}

/**
 * Comments written across a review, collected until they are sent as one.
 *
 * **Collect, then send** is the whole design, and it is about the receiving end
 * rather than the sending one. Firing a prompt per thought interrupts an agent
 * five times over ten minutes and produces five turns, each one re-reading the
 * files the last one just touched; the same five notes delivered together are one
 * turn against one snapshot of the branch. It also matches how reviewing actually
 * goes — the notes are made while reading and the decision to send is a separate,
 * later one.
 *
 * Persisted on every write, because the app holding it can go away between the
 * two. On a phone that is near-certain: an unsent comment lost to a process death
 * is worse than no comment feature at all.
 *
 * A separate object from `ChangesStore` rather than more fields on its state,
 * because its lifetime is different in the way that matters: everything on that
 * store is derived from the daemon and can be thrown away and re-read, while a
 * comment is the only thing on this screen that a person typed and that nothing
 * else in the world has a copy of.
 */
class ReviewCommentQueue(
    private val ref: ReviewRef,
    private val storage: ReviewStorage,
    /**
     * Hand one message to one pane. Null means it went; a [Trouble] means it did
     * not, in words a person can read.
     *
     * The one platform-specific line in this file. See the note at the top.
     */
    private val deliver: suspend (ReviewAgentTarget, String) -> Trouble?,
    /** Injected so a test can assert on a receipt's time. Seconds since the epoch. */
    private val now: () -> Long = { System.currentTimeMillis() / 1_000 },
) {
    /**
     * Everything about the queue, as one value, for the reason
     * [ChangesState] gives at length: a Compose screen collecting four flows
     * would redraw four times over one send and could catch it half done.
     */
    data class State(
        /** Written, not yet sent. */
        val pending: List<ReviewComment> = emptyList(),
        /**
         * The last few batches that went, newest first.
         *
         * Capped at [SENT_KEPT], because this is a receipt and not a history: the
         * question it answers is "what did I just send", asked within a minute of
         * sending it.
         */
        val sent: List<SentReviewBatch> = emptyList(),
        /**
         * Set while a send is in flight, so the button can say so and cannot be
         * pressed twice — the one way this app could produce a duplicate prompt
         * on a protocol that has no way to notice one.
         */
        val sending: Boolean = false,
        /**
         * A send that did not go.
         *
         * NOT cleared by anything on a timer and never acted on automatically.
         * There is no acknowledgment on this path — see [SentReviewBatch] — so an
         * automatic retry would be the app deciding, with no evidence, that a
         * prompt which may well have arrived should be delivered a second time.
         * The comments stay in [pending] and the reader is the one who decides.
         */
        val failure: Trouble? = null,
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    init {
        val raw = storage.read(key)
        val stored = raw?.let {
            runCatching { json.decodeFromString(Stored.serializer(), it) }.getOrNull()
        }
        if (stored != null) {
            _state.value = State(pending = stored.pending, sent = stored.sent)
        }
    }

    fun add(comment: ReviewComment) {
        // A new comment is evidence the last failure has been read and moved
        // past. Left up, it would sit above a queue it no longer describes.
        update { it.copy(pending = it.pending + comment, failure = null) }
    }

    /** The comment a composer produces, timestamped by this queue's clock. */
    fun write(anchor: ReviewAnchor, text: String) {
        val trimmed = text.trim()
        if (trimmed.isEmpty()) return
        add(ReviewComment(anchor = anchor, text = trimmed, writtenAt = now()))
    }

    fun remove(comment: ReviewComment) {
        update { state -> state.copy(pending = state.pending.filterNot { it.id == comment.id }) }
    }

    fun clearFailure() {
        updateLive { it.copy(failure = null) }
    }

    /**
     * Everything queued, as one message.
     *
     * Numbered and grouped in the order they were written, which is reading order
     * — the order the reader went through the diff in, and therefore the order in
     * which the notes make sense to each other.
     */
    fun message(branch: String): String {
        val pending = _state.value.pending
        val lines = mutableListOf<String>()
        val noun = if (pending.size == 1) "note" else "notes"
        lines += if (branch.isEmpty()) {
            "Review $noun from Far Cooler (${pending.size}):"
        } else {
            "Review $noun on `$branch` from Far Cooler (${pending.size}):"
        }
        lines += ""
        for ((index, comment) in pending.withIndex()) {
            lines += "${index + 1}. In ${comment.anchor.promptDescription}:"
            comment.anchor.quote?.let { lines += "   > $it" }
            lines += "   ${comment.text}"
            lines += ""
        }
        return lines.joinToString("\n").trim()
    }

    /**
     * Hand the batch to an agent, once.
     *
     * The call [deliver] makes is `terminal.agent_prompt`, the same one a typed
     * message makes, so a comment batch arrives in the transcript exactly as a
     * typed message does — there is no separate "review comment" channel to keep
     * in step, and the agent's own history shows what it was told.
     *
     * On failure the comments STAY pending and nothing is retried. The reader can
     * press Try again, and that is the only thing that ever sends this batch a
     * second time: `request_no_wait` means a failure here is "this client did not
     * get an answer", which is not the same as "the agent did not get the
     * prompt", and only a person can weigh the difference.
     */
    suspend fun send(target: ReviewAgentTarget, branch: String) {
        val before = _state.value
        if (before.pending.isEmpty() || before.sending) return
        val text = message(branch)
        val count = before.pending.size
        updateLive { it.copy(sending = true) }
        val trouble = try {
            deliver(target, text)
        } finally {
            updateLive { it.copy(sending = false) }
        }
        if (trouble != null) {
            updateLive { it.copy(failure = trouble) }
            return
        }
        // The receipt goes in with the same write that empties the queue. On iOS
        // those are two `didSet`s and the note there says a crash between them
        // loses the receipt rather than the comments; one atomic write makes
        // that ordering unnecessary rather than merely safe.
        val receipt = SentReviewBatch(
            text = text, agentName = target.name, sentAt = now(), count = count
        )
        update {
            it.copy(
                pending = emptyList(),
                sent = (listOf(receipt) + it.sent).take(SENT_KEPT),
                failure = null,
            )
        }
    }

    /**
     * A change to something that is written down, and every one of them writes.
     *
     * Kotlin has no `didSet`, which on iOS is what guarantees no mutation can
     * skip the save. Two private mutators are this platform's version of that
     * guarantee: `_state.value = …` appears exactly twice in this file, and only
     * the pair below can reach it.
     */
    private fun update(transform: (State) -> State) {
        val next = transform(_state.value)
        _state.value = next
        save(next)
    }

    /**
     * A change to something that lives only as long as the screen does.
     *
     * [State.sending] and [State.failure] are not in [Stored] — a send in flight
     * cannot survive the process it is in flight on, and a failure restored from
     * disk would be a warning about a request nobody made this session. Writing
     * for them would be a preferences round trip that stores the same bytes.
     */
    private fun updateLive(transform: (State) -> State) {
        _state.value = transform(_state.value)
    }

    private fun save(state: State) {
        val raw = runCatching {
            json.encodeToString(Stored.serializer(), Stored(state.pending, state.sent))
        }.getOrNull() ?: return
        storage.write(key, raw)
    }

    private val key: String get() = "changes.comments.${ref.key}"

    @Serializable
    private data class Stored(
        val pending: List<ReviewComment> = emptyList(),
        val sent: List<SentReviewBatch> = emptyList(),
    )

    companion object {
        private const val SENT_KEPT = 5

        private val json = Json { ignoreUnknownKeys = true }
    }
}
