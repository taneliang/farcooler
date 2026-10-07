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

    override suspend fun page(before: Long?, limit: Int): JsonObject {
        if (before != null) olderCalls.incrementAndGet() else pageCalls.incrementAndGet()
        return pages.receive().getOrThrow()
    }

    override suspend fun follow(epoch: Long, afterRev: Long, waitMs: Int): JsonObject {
        followCalls.incrementAndGet()
        return follows.receive().getOrThrow()
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

    fun turn(ord: Long, prompt: String, origin: String = "Typed", outcome: String? = "Finished") = row(
        "turn:$ord", ord, "Turn",
        obj(
            "prompt" to text(prompt), "origin" to text(origin),
            "started_ms" to JsonPrimitive(1_000), "ended_ms" to JsonPrimitive(5_000), "duration_ms" to JsonPrimitive(4_000),
            "background_running" to JsonPrimitive(0),
        ).let { base -> if (outcome == null) base else JsonObject(base + ("outcome" to text(outcome))) },
    )

    fun prose(ord: Long, body: String) = row("prose:$ord", ord, "Prose", obj("text" to text(body), "conclusion" to JsonPrimitive(true)))

    fun page(epoch: Long, rev: Long, vararg rows: JsonObject, more: Boolean = false) = obj(
        "epoch" to JsonPrimitive(epoch), "rev" to JsonPrimitive(rev), "moreBefore" to JsonPrimitive(more),
        "rows" to kotlinx.serialization.json.JsonArray(rows.toList()),
    )
}
