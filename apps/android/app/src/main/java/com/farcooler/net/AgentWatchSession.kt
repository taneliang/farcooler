package com.farcooler.net

import com.farcooler.core.AgentWatchTransport
import com.farcooler.core.TerminalGrid
import com.farcooler.core.VtCore
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** What an emulator does for a watch: take bytes, show a grid. A seam so a JVM test needs no native core. */
interface WatchScreen {
    fun feed(bytes: ByteArray)
    fun snapshot(): TerminalGrid?
    fun free()
}

private class VtScreen(columns: Int, rows: Int) : WatchScreen {
    private val core = VtCore(columns, rows)
    override fun feed(bytes: ByteArray) = core.feed(bytes)
    override fun snapshot(): TerminalGrid? = core.snapshot()
    override fun free() = core.free()
}

/**
 * A running engine agent's terminal, to look at and nothing more (ov-976).
 *
 * The bytes come from [AgentWatchTransport], which can start and stop a stream and nothing else.
 * Whatever the emulator wants to say back (a cursor-position report, a mouse reply) is dropped here:
 * this class has no path to the agent at all, and the emulator's replies are never even read.
 * The stream opens with the pane's size as the runner's own `farcooler-size` marker, so the emulator
 * is built at the agent's columns and rows and never asks for a resize.
 */
class AgentWatchSession(
    private val workspace: String,
    private val item: String,
    private val transport: AgentWatchTransport,
    dispatcher: CoroutineDispatcher,
    private val makeScreen: (Int, Int) -> WatchScreen = { c, r -> VtScreen(c, r) },
) {
    sealed interface Phase {
        data object Connecting : Phase
        data object Live : Phase
        /** The agent's pane was reaped: the work moved on. */
        data object Finished : Phase
        /** The link dropped while the agent was still running. */
        data object Lost : Phase
        /** The agent isn't running (a review, a merge, one that already ended), or can't be reached. */
        data class Unavailable(val gotPicture: Boolean) : Phase
    }

    private val scope = CoroutineScope(SupervisorJob() + dispatcher)
    private val _phase = MutableStateFlow<Phase>(Phase.Connecting)
    val phase: StateFlow<Phase> = _phase.asStateFlow()
    private val _grid = MutableStateFlow<TerminalGrid?>(null)
    val grid: StateFlow<TerminalGrid?> = _grid.asStateFlow()

    private var screen: WatchScreen? = null
    private var pending = ByteArray(0)
    private var started = false

    fun start() {
        if (started) return
        started = true
        scope.launch {
            val opened = transport.startWatch(
                workspace, item,
                onChunk = { bytes -> scope.launch { consume(bytes) } },
                onEnd = { why -> scope.launch { ended(why) } },
            )
            if (!opened) _phase.value = Phase.Unavailable(gotPicture = false)
        }
    }

    private fun consume(bytes: ByteArray) {
        val live = screen
        if (live != null) { feed(live, bytes); return }
        // Until the marker has arrived the screen can't be sized; hold bytes (a marker is under 40).
        pending += bytes
        val (size, rest) = SizeMarker.split(pending) ?: run {
            if (pending.size > SizeMarker.LIMIT) { build(80, 24, pending); pending = ByteArray(0) }
            return
        }
        pending = ByteArray(0)
        build(size.first, size.second, rest)
    }

    private fun build(columns: Int, rows: Int, rest: ByteArray) {
        val made = makeScreen(columns.coerceIn(1, 500), rows.coerceIn(1, 300))
        screen = made
        feed(made, rest)
        _phase.value = Phase.Live
    }

    private fun feed(live: WatchScreen, bytes: ByteArray) {
        live.feed(bytes)
        // The emulator's replies to the program (cursor reports, mouse) are NOT read: nothing goes back.
        _grid.value = live.snapshot()
    }

    private fun ended(why: String?) {
        val saw = screen != null
        _phase.value = when {
            why == AgentWatchTransport.LOST && saw -> Phase.Lost
            why == null && saw -> Phase.Finished
            else -> Phase.Unavailable(saw)
        }
    }

    fun stop() {
        scope.launch { transport.stopWatch(workspace, item) }.invokeOnCompletion {
            screen?.free(); screen = null
            scope.cancel()
        }
    }

    companion object {
        /** What to tell the owner. Nothing while it runs. */
        fun status(phase: Phase): String? = when (phase) {
            Phase.Connecting, Phase.Live -> null
            Phase.Finished -> "Finished"
            Phase.Lost -> "Lost the connection to this agent."
            is Phase.Unavailable -> if (phase.gotPicture) "Lost the connection to this agent." else "This agent isn’t running right now."
        }
    }
}

/** The runner's own size marker, `ESC P > farcooler-size;<columns>;<rows> ESC \`, at the head of a watch. */
object SizeMarker {
    const val LIMIT = 64
    private const val HEAD = "\u001bP>farcooler-size;"

    /** The pane's size and the bytes after the marker, or null if the marker hasn't fully arrived. */
    fun split(bytes: ByteArray): Pair<Pair<Int, Int>, ByteArray>? {
        val text = String(bytes, Charsets.ISO_8859_1)
        if (!text.startsWith(HEAD)) return null
        val end = text.indexOf("\u001b\\")
        if (end < 0) return null
        val parts = text.substring(HEAD.length, end).split(';')
        val columns = parts.getOrNull(0)?.toIntOrNull() ?: return null
        val rows = parts.getOrNull(1)?.toIntOrNull() ?: return null
        return (columns to rows) to bytes.copyOfRange(end + 2, bytes.size)
    }
}
