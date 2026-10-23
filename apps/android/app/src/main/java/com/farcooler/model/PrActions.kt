package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive

// A phone acting on a pull request as the runner's own `gh` login (ov-317, team
// mode T6): mark a lane's draft pull request ready for review, and reply to a
// review thread an agent escalated. The runner does both (`pr.ready` and
// `pr.thread_reply`, `pr_actions`, Control scope); this holds what a phone sends,
// what it asks before sending, and what it says when the runner refuses.
// AgentKit's `PhonePrActions`, word for word but for the case: Material's is
// sentence case.
//
// Review on GitHub is not here: it is [GitHubLink.url], opened by the system,
// which hands a github.com link to GitHub when it is installed.

/**
 * Whether this device may act on pull requests on the runner it reached: the runner has them, this device holds
 * Control, and it isn't someone else's (ov-712). Both post as the runner's own `gh` login, the owner's, which the
 * runner refuses a member (ov-637). [viewer] is `member.list`'s `you`, as Keep's is ([RulingActions.canMark]).
 */
fun DaemonBuild.mayActOnPullRequests(viewer: String?): Boolean =
    can(Capability.PR_ACTIONS) && grantedScope != "read" && viewer.isNullOrEmpty()

object PrActions {
    const val READY_METHOD = "pr.ready"
    const val REPLY_METHOD = "pr.thread_reply"

    /** The longest reply, in characters: the runner's cap (`pr_actions::REPLY_MAX`). */
    const val REPLY_MAX = 2000

    const val MARK_READY = "Mark ready"
    /** Somebody else's pull request, on a Needs You row: reviewed there. */
    const val REVIEW_ON_GITHUB = "Review on GitHub"

    /** A thread, and a lane's own pull request: opened there. */
    const val OPEN_ON_GITHUB = "Open on GitHub"
    const val REPLY = "Reply"
    const val SEND_REPLY = "Send reply"

    /** What it does: reviewers are asked the moment it is ready, so it asks first. */
    const val READY_MESSAGE = "It stops being a draft, and the people asked to review it are notified."

    /** `pr.ready`'s arguments: the lane, and the number of the draft the row showed. */
    fun readyRequest(lane: String, number: Int): List<Pair<String, Any>> = listOf("lane" to lane, "pr_number" to number)

    /** `pr.thread_reply`'s arguments for the card whose Needs You row is the thread; with [send] false the runner posts nothing. */
    fun replyRequest(task: String, body: String, send: Boolean, shown: ReplyPreview? = null): List<Pair<String, Any>> =
        listOf("task" to task, "body" to body, "send" to send) +
            (if (shown != null) listOf("note_id" to shown.noteId, "digest" to shown.digest) else emptyList())

    /** The length the runner counts: Unicode scalar values (`pr_actions::REPLY_MAX`), not UTF-16 units. */
    fun length(text: String): Int = text.codePointCount(0, text.length)

    /** A reply, trimmed, or null when it is empty or past the cap. */
    fun reply(text: String): String? = text.trim().takeIf { it.isNotEmpty() && length(it) <= REPLY_MAX }

    /** The confirm sheet's title for Mark ready. */
    fun readyTitle(stage: PlanPrStage): String =
        if (stage.prNumber > 0) "Mark PR ${stage.prNumber} ready for review?" else "Mark ready for review?"

    /** Where a reply would go and as whom: `pr.thread_reply` with `send` false. */
    data class ReplyPreview(
        /** `owner/name`. */
        val repository: String,
        val number: Int,
        /** The runner's `gh` login: the name the reply is posted under. */
        val login: String,
        /** In the comment's thread (true), or on the pull request (false). */
        val inThread: Boolean,
        /** The waiting question the thread is on, and a digest of where its link resolves: handed back with the send. */
        val noteId: String = "",
        val digest: String = "",
    ) {
        /** "acme/billing #213". */
        val pullRequest: String get() = "$repository #$number"

        /** The confirm sheet's title: which pull request, and nothing else. */
        val title: String get() = "Reply on $pullRequest?"

        /** What the confirm sheet says it will do, naming the login. */
        val message: String
            get() = if (inThread) "Posts your reply in this thread as $login on GitHub."
            else "Posts your reply on the pull request as $login on GitHub."

        companion object {
            /** From the runner's answer; null for anything that does not name all three. */
            fun decode(o: JsonObject): ReplyPreview? {
                val repository = o["repository"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotEmpty() } ?: return null
                val number = o["number"]?.jsonPrimitive?.intOrNull?.takeIf { it > 0 } ?: return null
                val login = o["login"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotEmpty() } ?: return null
                val inThread = o["in_thread"]?.jsonPrimitive?.contentOrNull == "true"
                val noteId = o["note_id"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotEmpty() } ?: return null
                val digest = o["digest"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotEmpty() } ?: return null
                return ReplyPreview(repository, number, login, inThread, noteId, digest)
            }

            /** [decode] from the runner's bytes. */
            fun decode(text: String): ReplyPreview? =
                runCatching { Json.parseToJsonElement(text) as? JsonObject }.getOrNull()?.let(::decode)
        }
    }

    /** The excerpt of a long reply the confirmation shows; the whole of it is sent. */
    fun excerpt(text: String): String = if (text.length <= 280) text else text.take(279) + "…"

    /**
     * The sentence for a refusal: by the runner's `what` where it named one, else by its code word. [action] finishes
     * "Couldn’t …". Never the runner's own words.
     */
    fun refusal(word: String?, what: String?, action: String): String = when {
        what == "lane_pr" -> "This lane has no draft pull request to mark ready. It may already be ready, or closed."
        what == "thread_link" -> "Couldn’t find that thread on GitHub. Open it there to reply."
        what == "already_answered" -> "This thread was already answered."
        what == "reply_body" -> "A reply has to be between 1 and $REPLY_MAX characters."
        word == "pr-state-unavailable" ->
            "GitHub didn’t answer on the runner. Check that its GitHub login still works, then try again."
        word == "scope-denied" -> "This device can only look at this runner. Changing anything needs control."
        word == "capability-unsupported" -> "This runner’s Far Cooler is too old for this. Update it there, then try again."
        else -> "Couldn’t $action. Try again."
    }
}

/** Why a reply couldn't be previewed, as the sentence a screen shows. */
class PrActionFailure(val sentence: String) : Exception(sentence)
