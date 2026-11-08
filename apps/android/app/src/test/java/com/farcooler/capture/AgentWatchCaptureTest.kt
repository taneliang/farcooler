package com.farcooler.capture

import com.farcooler.core.AgentWatchTransport
import com.farcooler.ui.AgentWatchView
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The watch screen at the narrowest phone width, drawing the real bytes of a real agent pane
 * (tmux's own capture of a 120-column pane, `agent-watch-real-pane.bin`) through the real emulator
 * (ov-976). Needs the host JNI core: `-Pfarcooler.nativeLibDir=<target/debug>`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w320dp-h640dp-xxhdpi")
class AgentWatchCaptureTest {
    private val bytes = requireNotNull(javaClass.getResourceAsStream("/agent-watch-real-pane.bin")).readBytes()

    /** Delivers the pane's bytes as soon as it is asked, then ends the way [end] says. */
    private class Replay(val bytes: ByteArray, val end: String?, val ends: Boolean) : AgentWatchTransport {
        override suspend fun startWatch(workspace: String, item: String, onChunk: (ByteArray) -> Unit, onEnd: (String?) -> Unit): Boolean {
            if (bytes.isNotEmpty()) bytes.toList().chunked(700).forEach { onChunk(it.toByteArray()) }
            if (ends) onEnd(end)
            return true
        }
        override suspend fun stopWatch(workspace: String, item: String) {}
    }

    private fun shot(name: String, replay: Replay) {
        assumeTrue("needs the host JNI core", com.farcooler.core.NativeLibrary.loaded)
        Capture.both("agent-watch-$name", settleMs = 600) {
            AgentWatchView("ws", "ov-976", replay) {}
        }
    }

    @Test fun `running`() = shot("running", Replay(bytes, null, ends = false))
    @Test fun `finished`() = shot("finished", Replay(bytes, null, ends = true))
    @Test fun `lost`() = shot("lost", Replay(bytes, AgentWatchTransport.LOST, ends = true))
    @Test fun `not running`() = shot("not-running", Replay(ByteArray(0), null, ends = true))
}
