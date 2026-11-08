package com.farcooler.core

/**
 * What watching an engine agent's terminal asks of a runner's connection (ov-976).
 *
 * Two verbs, and neither can type: start the read-only stream, and stop it. There is deliberately
 * no `write`, `resize` or `tell` here, so a view holding only this has no way to put a byte in the
 * agent's pane. [ClientCore] is the real implementation; a JVM test passes a fake and asserts the
 * calls it saw.
 */
interface AgentWatchTransport {
    /**
     * Start streaming the agent for [item] in [workspace]. [onEnd] gets null when the agent's pane
     * was reaped, [LOST] when the link dropped first, and the runner's words when it could not be
     * opened. False means there is no session to open a channel on.
     */
    suspend fun startWatch(workspace: String, item: String, onChunk: (ByteArray) -> Unit, onEnd: (String?) -> Unit): Boolean

    /** Stop the stream [startWatch] opened. */
    suspend fun stopWatch(workspace: String, item: String)

    companion object {
        const val LOST = "lost"
    }
}
