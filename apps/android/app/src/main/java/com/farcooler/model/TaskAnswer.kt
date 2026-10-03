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
