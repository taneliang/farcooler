package com.farcooler.net

import com.farcooler.model.AgentRowFixture
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject

/**
 * A runner's rows, answered by the test: a call takes the next queued answer and
 * waits for one when there is none, as a follow held up on the runner does, so
 * ordering is driven by what the test queues and never by a clock.
 */
class FakeRowSource : AgentRowSource {
    val pages = Channel<Result<JsonObject>>(Channel.UNLIMITED)
    val follows = Channel<Result<JsonObject>>(Channel.UNLIMITED)
    val pageCalls = AtomicInteger()
    val followCalls = AtomicInteger()
    val olderCalls = AtomicInteger()

    /** Follow calls made and not yet ended: one that was cancelled has left once this drops. */
    val followsOut = AtomicInteger()

    /**
     * How long a follow call lingers between being made and waiting for its answer
     * (0 for none). A loop that has been stopped can sit there, and a test sets it
     * to make that gap wide enough to hit on purpose rather than once in a while.
     */
    @Volatile
    var beforeWaitMs = 0L

    /** The sources of the subagents this pane's runner serves, by `agentId` (ov-641): what [subagent] answers. */
    val children = java.util.concurrent.ConcurrentHashMap<String, FakeRowSource>()

    override fun subagent(agentId: String): AgentRowSource? = children[agentId]

    override suspend fun page(before: Long?, limit: Int): JsonObject {
        if (before != null) olderCalls.incrementAndGet() else pageCalls.incrementAndGet()
        return pages.receive().getOrThrow()
    }

    override suspend fun follow(epoch: Long, afterRev: Long, waitMs: Int): JsonObject {
        followCalls.incrementAndGet()
        followsOut.incrementAndGet()
        try {
            if (beforeWaitMs > 0) Thread.sleep(beforeWaitMs)
            return follows.receive().getOrThrow()
        } finally {
            followsOut.decrementAndGet()
        }
    }

    fun answerPage(body: JsonObject = AgentRowFixture.page) = pages.trySend(Result.success(body))
    fun answerFollow(body: JsonObject = AgentRowFixture.follow) = follows.trySend(Result.success(body))
    fun failFollow(error: Throwable) = follows.trySend(Result.failure(error))
    fun failPage(error: Throwable) = pages.trySend(Result.failure(error))

    /** A follow that found nothing new, as a held call that timed out. */
    fun answerNothing(epoch: Long = 4, rev: Long = 12) = answerFollow(
        buildJsonObject {
            put("epoch", JsonPrimitive(epoch))
            put("rev", JsonPrimitive(rev))
            put("reset", JsonPrimitive(false))
            put("changes", kotlinx.serialization.json.JsonArray(emptyList()))
        },
    )
}

/** A scope the loops run on, and a way to end them all. */
class TestScope {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    fun close() = scope.cancel()
}

/**
 * Wait for [condition], polling and returning as soon as it holds. Never bounded
 * under 30 s: a slow CI runner is not a failure, and a green run doesn't wait.
 */
fun eventually(what: String, timeoutMs: Long = 30_000, condition: () -> Boolean) {
    val deadline = System.nanoTime() + timeoutMs * 1_000_000
    while (!condition()) {
        if (System.nanoTime() > deadline) throw AssertionError("Timed out waiting for $what")
        Thread.sleep(2)
    }
}

/** Rows and pages as the client core writes them, for tests that need a few of their own. */
object RowJson {
    private fun obj(vararg pairs: Pair<String, kotlinx.serialization.json.JsonElement>) = buildJsonObject {
        pairs.forEach { (k, v) -> put(k, v) }
    }

    private fun text(value: String) = JsonPrimitive(value)

    private fun row(id: String, ord: Long, kind: String, payload: JsonObject) = obj(
        "id" to text(id), "ord" to JsonPrimitive(ord), "rev" to JsonPrimitive(ord), "provisional" to JsonPrimitive(false),
        "kind" to obj(kind to payload),
    )

    /** [activity] (`Busy`, `Waiting`, ...) is the newest turn's, which the registry says while it's open. */
    fun turn(
        ord: Long, prompt: String, origin: String = "Typed", outcome: String? = "Finished", activity: String? = null,
        suggestion: String? = null, images: List<String> = emptyList(), startedMs: Long = 1_000,
    ) = row(
        "turn:$ord", ord, "Turn",
        obj(
            "prompt" to text(prompt), "origin" to text(origin),
            "started_ms" to JsonPrimitive(startedMs), "ended_ms" to JsonPrimitive(startedMs + 4_000), "duration_ms" to JsonPrimitive(4_000),
            "background_running" to JsonPrimitive(0),
        ).let { base -> if (outcome == null) base else JsonObject(base + ("outcome" to text(outcome))) }
            .let { base -> if (activity == null) base else JsonObject(base + ("activity" to text(activity))) }
            .let { base -> if (suggestion == null) base else JsonObject(base + ("suggestion" to text(suggestion))) }
            .let { base ->
                if (images.isEmpty()) base
                else JsonObject(base + ("images" to kotlinx.serialization.json.JsonArray(images.map { obj("mime" to text(it)) })))
            },
    )

    /** A finished tool call that opens to its [input] and [result] (ov-452). */
    fun tool(ord: Long, name: String, summary: String, input: String? = null, result: String? = null, status: String = "Done") = row(
        "tool:$ord", ord, "Tool",
        obj(
            "name" to text(name), "summary" to text(summary), "status" to text(status),
            "started_ms" to JsonPrimitive(1_000 + ord * 1_000), "ended_ms" to JsonPrimitive(1_400 + ord * 1_000),
            "diff" to kotlinx.serialization.json.JsonArray(emptyList()),
        ).let { base -> if (input == null) base else JsonObject(base + ("input" to text(input))) }
            .let { base -> if (result == null) base else JsonObject(base + ("result" to text(result))) },
    )

    /** The agent's task list (ov-452): subject to `Pending`, `InProgress` or `Completed`. */
    fun tasks(ord: Long, vararg items: Pair<String, String>) = row(
        "tasks:$ord", ord, "Tasks",
        obj("items" to kotlinx.serialization.json.JsonArray(items.map { obj("subject" to text(it.first), "status" to text(it.second)) })),
    )

    fun thinking(ord: Long) = row("think:$ord", ord, "Thinking", obj("started_ms" to JsonPrimitive(1_000 + ord * 1_000), "ended_ms" to JsonPrimitive(1_900 + ord * 1_000)))

    /** claude's `Try "…"` example (ov-409): the one `Hint` row, its text empty once the box shows something else. */
    fun hint(ord: Long, words: String) = row("hint:composer", ord, "Hint", obj("text" to text(words)))

    /** A held ask (ov-370): `question`, `plan` or `permission`, held under `hook-ask-1` unless [held] is null. */
    fun heldAsk(ord: Long, which: String, held: String? = "hook-ask-1", answeredBy: String? = null): JsonObject {
        val base = when (which) {
            "question" -> obj(
                "kind" to text("Question"), "text" to text("Which color should the button be?"), "tool" to text("AskUserQuestion"),
                "questions" to kotlinx.serialization.json.JsonArray(listOf(obj(
                    "question" to text("Which color should the button be?"), "header" to text("Color"), "multi_select" to JsonPrimitive(false),
                    "options" to kotlinx.serialization.json.JsonArray(listOf(
                        obj("label" to text("Red"), "description" to text("Warm and loud")),
                        obj("label" to text("Blue"), "description" to text("Calm and quiet")),
                    )),
                ))),
            )
            "plan" -> obj(
                "kind" to text("PlanExit"), "text" to text("# Plan 1. Make the button blue."), "tool" to text("ExitPlanMode"),
                "plan" to text("# Plan\n\n1. Make the button blue.\n2. Ship it."),
            )
            else -> obj("kind" to text("Permission"), "text" to text("Bash touch spike-made-this.txt"), "tool" to text("Bash"))
        }
        var payload = JsonObject(base + ("answered" to JsonPrimitive(false)))
        if (held != null) payload = JsonObject(payload + ("held" to text(held)))
        if (answeredBy != null) payload = JsonObject(payload + ("answered_by" to text(answeredBy)))
        return row("ask:$ord", ord, "Ask", payload)
    }

    /** A subagent's row in its parent's rows (ov-453): `agent_id` once the runner has named it. */
    fun subagent(
        ord: Long, call: String, agentId: String?, type: String = "general-purpose", description: String = "Review ov-616",
        status: String = "Running", startedMs: Long = 1_000,
    ) = row(
        "sub:$call", ord, "Subagent",
        obj(
            "tool_use_id" to text(call), "agent_type" to text(type), "description" to text(description), "background" to JsonPrimitive(true),
            "status" to text(status), "started_ms" to JsonPrimitive(startedMs), "tool_count" to JsonPrimitive(3),
            "current_action" to text("Read x"), "last_ms" to JsonPrimitive(2_000),
        ).let { base -> if (agentId == null) base else JsonObject(base + ("agent_id" to text(agentId))) },
    )

    fun prose(ord: Long, body: String) = row("prose:$ord", ord, "Prose", obj("text" to text(body), "conclusion" to JsonPrimitive(true)))

    fun page(epoch: Long, rev: Long, vararg rows: JsonObject, more: Boolean = false) = obj(
        "epoch" to JsonPrimitive(epoch), "rev" to JsonPrimitive(rev), "moreBefore" to JsonPrimitive(more),
        "rows" to kotlinx.serialization.json.JsonArray(rows.toList()),
    )
}
