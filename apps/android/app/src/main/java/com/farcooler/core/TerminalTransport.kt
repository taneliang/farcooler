package com.farcooler.core

import kotlinx.serialization.json.JsonObject

/**
 * The three things a terminal pane asks of its runner's connection: a call, and
 * starting and stopping its byte stream.
 *
 * [ClientCore] is the one real implementation. This exists so that
 * `TerminalSession`'s stream recovery, which decides when channels open and
 * close, runs in a JVM test against a fake: the core is JNI and cannot.
 */
interface TerminalTransport {
    suspend fun call(method: String, args: JsonObject = JsonObject(emptyMap())): JsonObject

    /** Whether a stream opened. False is an answer, not an error: there is no session to open one on. */
    suspend fun startStream(terminal: String, onChunk: (ByteArray) -> Unit, onEnd: (String?) -> Unit): Boolean

    suspend fun stopStream(terminal: String)
}
