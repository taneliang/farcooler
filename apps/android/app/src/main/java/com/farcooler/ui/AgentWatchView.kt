package com.farcooler.ui

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
import androidx.compose.runtime.remember
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
                TerminalCanvas(
                    grid = shown, fontFamily = FontFamily.Monospace, fontSize = 13f,
                    onSize = { _, _ -> }, onTap = {}, onLongPress = { _, _ -> }, onScroll = { _, _, _ -> },
                )
            } else {
                Text(status ?: "Connecting…", Modifier.align(Alignment.Center).padding(16.dp))
            }
        }
    }
}
