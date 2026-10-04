package com.farcooler.net

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.farcooler.core.TerminalTransport
import com.farcooler.core.VtCore
import com.farcooler.model.TaskKeyIndex
import com.farcooler.model.TaskKeyLinks
import com.farcooler.model.TaskKeyTarget
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith

/**
 * A long press's link on real terminal bytes (ov-215, review 1004j X3): the
 * bytes fed into the real core, and [TerminalSession.linkAt] asking it for the
 * URL and the word under a cell, as the pane's long press does.
 *
 * On a device for `NativeBridgeTest`'s reason: a desktop JVM has no
 * `libfarcooler_jni.so`. `TerminalPressTest` covers the pane's calls on the
 * JVM with a stand-in for the core.
 */
@RunWith(AndroidJUnit4::class)
class TerminalTaskKeyDeviceTest {
    private object NoRunner : TerminalTransport {
        override suspend fun call(method: String, args: JsonObject): JsonObject = JsonObject(emptyMap())
        override suspend fun startStream(terminal: String, onChunk: (ByteArray) -> Unit, onEnd: (String?) -> Unit) = false
        override suspend fun stopStream(terminal: String) {}
    }

    private val index = TaskKeyIndex("r1", setOf("ov"), mapOf("ov-190" to TaskKeyTarget("r1", "w", "t-190", "ov-190")))

    @Test
    fun aKeyInOutputIsATaskLinkAndAUrlStaysTheUrl() {
        val vt = VtCore(80, 24)
        vt.feed("done in ov-190.\r\nhttps://x.dev/ov-190\r\n".toByteArray())
        val session = TerminalSession("t", NoRunner, links = vt)
        try {
            assertEquals(TaskKeyLinks.url("r1", "ov-190"), session.linkAt(0, 9, index))
            assertNull("the word \"done\" is no key", session.linkAt(0, 3, index))
            assertNull("a blank cell pastes", session.linkAt(0, 40, index))
            assertNull("no board read", session.linkAt(0, 9, TaskKeyIndex.EMPTY))
            assertEquals("a URL wins over the key inside it", "https://x.dev/ov-190", session.linkAt(1, 16, index))
        } finally {
            session.dispose()
            vt.free()
        }
    }
}
