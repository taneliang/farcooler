package com.farcooler.core

import android.util.Base64
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import java.util.concurrent.Executors
import java.util.concurrent.ConcurrentHashMap

/** What the core refused, in a sentence written for a human. */
open class CoreException(message: String) : Exception(message)

/**
 * The link is gone, as opposed to the request being refused.
 *
 * Answered by the core rather than worked out from the message here: Rust
 * still has the error's type at the moment it is produced, and
 * [com.farcooler.net.Connection.Failure] matching substrings is a compromise
 * the connect path makes because a connect failure genuinely arrives as prose.
 * A call on a live session need not make it.
 */
class DisconnectedException(message: String) : CoreException(message)

/**
 * Kotlin's view of the Rust client core.
 *
 * Everything about talking to a host — SSH, the protocol, the shapes of the
 * answers — happens on the other side of this file. What is left here is
 * turning a polling C API into something coroutines can suspend on.
 *
 * The core is asynchronous underneath and synchronous at its boundary, which is
 * deliberate: bridging two async runtimes through callbacks means one of them
 * is always wrong about which thread it is on. So calls hand back a ticket, a
 * pump drains finished results, and this class matches them to the coroutines
 * waiting for them.
 *
 * ## One thread, named
 *
 * The C ABI says a handle is not thread-safe and must be confined to one
 * thread. Kotlin has no `actor` keyword that would enforce that, so the
 * confinement is a single-thread dispatcher every native call hops onto, and
 * the handle is private to this object. The thread is named so it is
 * identifiable in a trace: a native crash on an anonymous pool thread tells you
 * nothing about who was calling.
 */
class ClientCore {
    private val dispatcher =
        Executors.newSingleThreadExecutor { runnable ->
            Thread(runnable, "farcooler-client").apply { isDaemon = true }
        }.asCoroutineDispatcher()

    /**
     * Written only on [dispatcher]; read from callers' threads by [stopStream]
     * and [drain]'s guard, so the write has to be published rather than left to
     * whenever the JIT feels like flushing it.
     */
    @Volatile
    private var handle: Long = 0

    /** Coroutines waiting on a ticket. */
    private val waiting = ConcurrentHashMap<Long, CompletableDeferred<JsonObject>>()

    /**
     * Progress reporters for in-flight image pastes, by ticket.
     *
     * Separate from [waiting] because a paste produces MANY lines under the
     * same ticket and only the last one is the answer. Completing on the first
     * would leave the transfer running with nobody listening to it.
     */
    private val reporting = ConcurrentHashMap<Long, (Long, Long) -> Unit>()

    /**
     * What a live terminal stream reports back.
     *
     * Not a continuation, because a stream is not an answer to anything: it
     * produces bytes until the pane ends or someone stops watching.
     */
    private class Stream(
        val onChunk: (ByteArray) -> Unit,
        /**
         * Null means the pane ended; a string is the reason it could not be
         * read. Either way this stream is over and has been forgotten here.
         */
        val onEnd: (String?) -> Unit,
    )

    private val streams = ConcurrentHashMap<String, Stream>()
    private var pump: Job? = null

    private val json = Json { ignoreUnknownKeys = true; isLenient = true }

    /** Whether a core exists at all. False only when the `.so` did not load. */
    val isUsable: Boolean get() = NativeLibrary.loaded

    private suspend fun ensureHandle(): Long = withContext(dispatcher) {
        if (!NativeLibrary.loaded) return@withContext 0L
        if (handle == 0L) handle = NativeClient.nativeNew()
        handle
    }

    /** Connect to a host. [config] is the JSON the core documents. */
    suspend fun connect(config: JsonObject): JsonObject =
        submit { NativeClient.nativeConnect(it, config.toString()) }

    /** Invoke a method. */
    suspend fun call(method: String, args: JsonObject = JsonObject(emptyMap())): JsonObject =
        submit { NativeClient.nativeCall(it, method, args.toString()) }

    /**
     * Paste a file into a terminal, and hand back the path it landed at.
     *
     * Its own entry point rather than a [call] method because the payload is
     * binary: the JSON boundary every other method uses would mean base64 in
     * both directions to carry bytes nothing in Kotlin looks at.
     */
    suspend fun pasteFile(
        terminal: String,
        name: String,
        mime: String,
        data: ByteArray,
        onProgress: (Long, Long) -> Unit,
    ): String {
        val result =
            submit(onProgress) { NativeClient.nativePasteFile(it, terminal, name, mime, data) }
        return result["path"]?.jsonPrimitive?.contentOrNull
            ?: throw CoreException("The client core returned something unreadable.")
    }

    suspend fun isConnected(): Boolean = withContext(dispatcher) {
        handle != 0L && NativeClient.nativeConnected(handle)
    }

    /**
     * Watch a terminal's output as it happens.
     *
     * The core opens a second SSH channel carrying nothing but this pane's
     * bytes — the same bytes tmux wrote, in the order it wrote them — so what
     * arrives here is what a terminal emulator eats, not a snapshot of the
     * screen after it settled. Cursor motion, redraws and animation exist on
     * the wire again, and the delay is one network round trip rather than
     * however long until the next poll.
     *
     * Returns false when there is no SSH session to open a channel on, which is
     * a real answer and not an error: the caller falls back to polling.
     */
    suspend fun startStream(
        terminal: String,
        onChunk: (ByteArray) -> Unit,
        onEnd: (String?) -> Unit,
    ): Boolean {
        val h = ensureHandle()
        if (h == 0L) return false
        startPumping()
        // Registered before starting, not after: the first chunk can be
        // delivered by a pump tick that lands between the two, and a stream
        // nothing is listening for yet would drop the replay of the screen —
        // the one chunk whose loss is visible, because it is the whole screen.
        streams[terminal] = Stream(onChunk, onEnd)
        val started = withContext(dispatcher) { NativeClient.nativeStreamStart(h, terminal) }
        if (!started) streams.remove(terminal)
        return started
    }

    /** Stop watching. Safe when nothing is running. */
    suspend fun stopStream(terminal: String) {
        streams.remove(terminal)
        val h = handle
        if (h == 0L) return
        withContext(dispatcher) { NativeClient.nativeStreamStop(h, terminal) }
    }

    /** End the session and release the handle. */
    suspend fun close() {
        pump?.cancel()
        pump = null
        withContext(dispatcher) {
            if (handle != 0L) {
                NativeClient.nativeFree(handle)
                handle = 0
            }
        }
        // Every waiter is told, rather than left suspended forever. A screen
        // awaiting a call on a connection that has been torn down would
        // otherwise show its spinner until the process ends.
        val stranded = waiting.values.toList()
        waiting.clear()
        stranded.forEach { it.completeExceptionally(CoreException("The connection was closed.")) }
        dispatcher.close()
    }

    private suspend fun submit(
        onProgress: ((Long, Long) -> Unit)? = null,
        start: (Long) -> Long,
    ): JsonObject {
        val h = ensureHandle()
        if (h == 0L) {
            throw CoreException(
                if (NativeLibrary.loaded) "The client core could not be started."
                else "This build has no Far Cooler core for this device's processor."
            )
        }
        startPumping()

        val result = CompletableDeferred<JsonObject>()
        // Registered before the ticket is known — impossible — so instead the
        // ticket is taken and the waiter is filed on the same dispatcher the
        // pump drains on. That ordering is what stops a result arriving before
        // anything is waiting for it, which on a fast local host is not rare.
        // The reporter is filed in the same hop, for the same reason: a paste
        // can report progress before this coroutine would otherwise resume.
        val ticket = withContext(dispatcher) {
            val ticket = start(h)
            if (ticket != 0L) {
                waiting[ticket] = result
                if (onProgress != null) reporting[ticket] = onProgress
            }
            ticket
        }
        if (ticket == 0L) throw CoreException("The client core returned something unreadable.")
        try {
            return result.await()
        } finally {
            reporting.remove(ticket)
        }
    }

    /**
     * Drain finished results into the coroutines waiting for them.
     *
     * 20 ms is well under a frame and far above the cost of one read of an
     * empty queue, which is what this almost always is.
     */
    private fun startPumping() {
        if (pump != null) return
        pump = CoroutineScope(dispatcher).launch {
            while (isActive) {
                drain()
                delay(if (streams.isEmpty()) IDLE_POLL_MS else STREAMING_POLL_MS)
            }
        }
    }

    private fun drain() {
        val h = handle
        if (h == 0L) return
        while (true) {
            val raw = NativeClient.nativePoll(h) ?: return
            val line = runCatching { json.parseToJsonElement(raw).jsonObject }.getOrNull() ?: continue

            // A stream line carries no ticket, because nothing asked for it.
            val terminal = line["stream"]?.jsonPrimitive?.contentOrNull
            if (terminal != null) {
                deliver(terminal, line)
                continue
            }

            val ticket = line["ticket"]?.jsonPrimitive?.long ?: continue

            // A progress line is not the answer. Reporting it must not take
            // the waiter, or the transfer would run on with nobody awaiting it.
            val progress = line["progress"] as? JsonObject
            if (progress != null) {
                val sent = progress["sent"]?.jsonPrimitive?.long ?: 0L
                val total = progress["total"]?.jsonPrimitive?.long ?: 0L
                reporting[ticket]?.invoke(sent, total)
                continue
            }

            val waiter = waiting.remove(ticket) ?: continue

            if (line["ok"]?.jsonPrimitive?.booleanOrNull == true) {
                val result = line["result"] as? JsonObject ?: JsonObject(emptyMap())
                waiter.complete(result)
            } else {
                val message =
                    line["error"]?.jsonPrimitive?.contentOrNull ?: "the host refused the request"
                val lost = line["disconnected"]?.jsonPrimitive?.booleanOrNull == true
                waiter.completeExceptionally(
                    if (lost) DisconnectedException(message) else CoreException(message)
                )
            }
        }
    }

    /**
     * Hand one stream line to whoever is watching that terminal.
     *
     * Called from inside [drain], so chunks reach the handler in exactly the
     * order the host produced them — which is the one property a byte stream
     * cannot do without.
     */
    private fun deliver(terminal: String, line: JsonObject) {
        val stream = streams[terminal] ?: return
        val chunk = line["chunk"]?.jsonPrimitive?.contentOrNull
        if (chunk != null) {
            val bytes = runCatching { Base64.decode(chunk, Base64.DEFAULT) }.getOrNull()
            if (bytes != null) stream.onChunk(bytes)
            return
        }
        // Anything that is not a chunk ends the stream: the pane finished, or
        // it could not be opened at all.
        streams.remove(terminal)
        stream.onEnd(line["error"]?.jsonPrimitive?.contentOrNull)
    }

    companion object {
        /**
         * How often to look, which is not the same question while a stream is
         * open.
         *
         * 20 ms is nothing next to a round trip to a host, so it is the right
         * price for noticing that a request finished. It is not nothing next to
         * a keystroke echoing back in 6 ms, though — a fixed 20 ms tick would
         * be most of the latency of the fast path it sits in front of, and
         * would make the stream feel like the polling it replaced. So the pump
         * runs hot exactly while something is streaming, and settles back the
         * moment nothing is.
         */
        private const val IDLE_POLL_MS = 20L
        private const val STREAMING_POLL_MS = 4L

        /**
         * Generate a new SSH identity for this device.
         *
         * On the core rather than in Kotlin for the reason nothing else in this
         * project implements cryptography: there is one place that does it, and
         * it is a library maintained by people who do this for a living.
         */
        fun generateKey(comment: String): Pair<String, String>? {
            if (!NativeLibrary.loaded) return null
            val payload = NativeClient.nativeGenerateKey(comment) ?: return null
            val parsed = runCatching {
                Json.parseToJsonElement(payload).jsonObject
            }.getOrNull() ?: return null
            val private = parsed["private_key"]?.jsonPrimitive?.contentOrNull ?: return null
            val public = parsed["public_key"]?.jsonPrimitive?.contentOrNull ?: return null
            return private to public
        }

        /**
         * The public key belonging to a private key.
         *
         * Derived rather than stored: a device has one identity, and keeping
         * the public half somewhere else means two facts that can disagree —
         * which they do, because a keystore and a preferences file do not have
         * the same lifetime.
         */
        fun publicKey(privateKey: String): String? {
            if (!NativeLibrary.loaded) return null
            return NativeClient.nativePublicKey(privateKey)
        }
    }
}
