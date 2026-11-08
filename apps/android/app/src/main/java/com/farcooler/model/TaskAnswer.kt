package com.farcooler.model

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * The one task write this app makes: an answer to an agent's question.
 *
 * The orchestrator owns the task list (ov-184). The phone reads the board and
 * answers decisions, and never creates, edits, re-statuses or deletes a task;
 * the client core refuses any `task.note` that is not an answer
 * (`crates/client/src/ffi.rs`, `task_note_of`), and has no arm for
 * `task.create`, `task.update` or `task.set_status`.
 *
 * Pure, so a JVM can prove it; `TaskAnswerTest` does.
 */
object TaskAnswer {
    /** The wire method an answer goes out on. */
    const val METHOD = "task.note"

    /**
     * `task.note`'s arguments for [body] as the answer to [task]'s question.
     * The runner writes it as the person, takes the decision off Needs You,
     * and wakes the agent waiting on it.
     */
    fun request(task: String, body: String): JsonObject = buildJsonObject {
        put("task", task)
        put("kind", "answer")
        put("body", body)
    }
}

/**
 * The owner's "Allow cutover once" (ov-974, ov-977): the one answer this phone signs, with its step-up key after a
 * fingerprint, face or screen lock, because only a signed one tells the runner the owner was there to say it. Every
 * other answer goes out plain, as before. AgentKit's `CutoverGrant`.
 */
object CutoverGrant {
    /** The option an orchestrator offers (`cutover_grant::ALLOW`). */
    const val ALLOW = "Allow cutover once"

    /** Whether [body] is the grant, as the runner reads it: the option's words, any case, trimmed. */
    fun isGrant(body: String): Boolean = body.trim().lowercase() == ALLOW.lowercase()

    /** What the prompt says. */
    fun reason(runner: String) = "Allow your orchestrator on $runner to run the cutover once."
}

/** A grant that didn't go, in the sentence to show; the person closing the prompt is an empty one. */
class GrantNotSent(val sentence: String) : Exception(sentence)
