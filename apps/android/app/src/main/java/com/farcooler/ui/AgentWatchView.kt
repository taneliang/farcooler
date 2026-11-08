package com.farcooler.ui

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.core.AgentWatchTransport
import com.farcooler.net.AgentWatchSession
import kotlinx.coroutines.asCoroutineDispatcher
import java.util.concurrent.Executors

internal const val WATCH_FONT_SIZE = 13f

/** The size a watched pane is drawn at: its grid at the font's own cell size, plus the canvas's padding. */
internal object AgentWatchLayout {
    fun contentSize(columns: Int, rows: Int, cellWidth: Float, cellHeight: Float, padding: Float): Pair<Float, Float> =
        (columns * cellWidth + padding * 2) to (rows * cellHeight + padding * 2)
}

/** Where the app supplies the connection that can watch an agent, per host. Absent: no Watch button. */
val LocalAgentWatch = compositionLocalOf<AgentWatchTransport?> { null }

/**
 * A running engine agent's terminal, to look at and nothing more (ov-976).
 *
 * The canvas is handed no-op callbacks for tap, long press and scroll and no size report, and the
 * session behind it holds a transport that can only start and stop the stream, so there is no
 * way for a touch to reach the agent. The picture scales to fit the width.
 */
@Composable
fun AgentWatchView(workspace: String, item: String, transport: AgentWatchTransport, close: () -> Unit) {
    val session = remember(workspace, item, transport) {
        val thread = Executors.newSingleThreadExecutor { r -> Thread(r, "farcooler-watch").apply { isDaemon = true } }
        AgentWatchSession(workspace, item, transport, thread.asCoroutineDispatcher())
    }
    DisposableEffect(session) {
        session.start()
        onDispose { session.stop() }
    }
    val grid by session.grid.collectAsStateWithLifecycle()
    val phase by session.phase.collectAsStateWithLifecycle()
    val status = AgentWatchSession.status(phase)
    Column(Modifier.fillMaxSize()) {
        Row(Modifier.padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("Watching $item", style = MaterialTheme.typography.titleMedium)
                Text(listOfNotNull("Read-only", status.takeIf { grid != null }).joinToString(" · "), style = MaterialTheme.typography.bodySmall)
            }
            TextButton(onClick = close) { Text("Close") }
        }
        Box(Modifier.weight(1f).fillMaxSize().semantics { contentDescription = "Agent terminal for $item, read-only" }) {
            val shown = grid
            if (shown != null) {
                // At the font's own size, never shrunk to fit: a pane wider than the phone scrolls sideways
                // (and a taller one up and down), so a 120-column pane stays readable.
                val measurer = rememberTextMeasurer()
                val density = LocalDensity.current
                val cell = remember(measurer) { measureCell(measurer, TextStyle(fontFamily = FontFamily.Monospace, fontSize = WATCH_FONT_SIZE.sp)) }
                val size = AgentWatchLayout.contentSize(shown.columns, shown.rows, cell.width, cell.height, with(density) { CellMetrics.padding.toPx() })
                Box(Modifier.fillMaxSize().horizontalScroll(rememberScrollState()).verticalScroll(rememberScrollState())) {
                    Box(Modifier.size(with(density) { size.first.toDp() }, with(density) { size.second.toDp() })) {
                        TerminalCanvas(
                            grid = shown, fontFamily = FontFamily.Monospace, fontSize = WATCH_FONT_SIZE,
                            onSize = { _, _ -> }, onTap = {}, onLongPress = { _, _ -> }, onScroll = { _, _, _ -> },
                        )
                    }
                }
            } else {
                Text(status ?: "Connecting…", Modifier.align(Alignment.Center).padding(16.dp))
            }
        }
    }
}
