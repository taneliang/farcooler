package com.farcooler.net

import android.util.Base64
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.farcooler.core.TerminalGrid
import com.farcooler.core.VtCore
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The scrolling bug, measured against the real emulator.
 *
 * This cannot be a JVM unit test for the reason `NativeBridgeTest` states:
 * there is no `libfarcooler_jni.so` on a desktop JVM's library path, and the
 * number this is about — the core's own history size — is the core's to report.
 * `TerminalScrollbackTest` covers the parts that are pure bytes.
 *
 * What it pins is the difference the fix makes, in the two numbers a swipe
 * depends on. Measured against the emulator this wraps — `crates/vt`, driven
 * directly, 80x24, a 24-line capture: without scrollback it reports
 * `historySize = 0` and `scroll(5)` leaves `displayOffset` at 0. With 100 lines
 * of history fed above the same capture and separated from it by a clear, it
 * reports `historySize = 100`, `scroll(5)` moves `displayOffset` to 5, and the
 * top row goes from "screen line 0" to "history line 95".
 */
@RunWith(AndroidJUnit4::class)
class TerminalScrollbackDeviceTest {

    /** As many lines as the screen is tall, separated the way a capture is. */
    private val capture = (0 until 24).joinToString("\n") { "screen line $it" }.toByteArray()

    /** As the host sends it: line feeds already repaired, ready to feed. */
    private val scrollback = (0 until 100).joinToString("") { "history line $it\r\n" }.toByteArray()

    @Test
    fun aCapturedScreenAloneHasNothingToSwipeInto() {
        // The bug, stated as a measurement rather than a description. A poll
        // carries `capture-pane -e -p` — the visible screen and no history —
        // and every enrolled device is on that path, because `crates/fence`
        // writes a forced SSH command and the stream never delivers a byte. Fed
        // into an emulator exactly as tall as the screen, there is nothing
        // above the top row for a swipe to reach.
        val vt = VtCore(80, 24)
        vt.feed(CapturedPane.feed(ByteArray(0), capture))

        val live = vt.snapshot()!!
        assertEquals("a screen-only capture leaves nothing above it", 0, live.historySize)
        assertEquals(0, live.displayOffset)

        vt.scroll(5)
        assertEquals(
            "a swipe on a pane with no history has nowhere to go",
            0,
            vt.snapshot()!!.displayOffset,
        )
        vt.free()
    }

    @Test
    fun scrollbackFedAboveTheScreenIsSomethingToSwipeInto() {
        val vt = VtCore(80, 24)
        vt.feed(CapturedPane.feed(scrollback, capture))

        val live = vt.snapshot()!!
        // The scrollback is above the screen rather than instead of it, which
        // is what the clear between the two captures buys — see
        // `aShortCaptureStillLandsAtTheTopOfTheScreen` for the case where
        // leaving it out is visibly wrong.
        assertTrue(
            "expected scrollback above the screen, got historySize=${live.historySize}",
            live.historySize > 0,
        )
        assertEquals(0, live.displayOffset)
        // And the screen is still the screen: the capture is what is showing,
        // with the history above it rather than in place of it.
        assertEquals("screen line 0", rowText(live, 0))

        vt.scroll(5)
        val back = vt.snapshot()!!
        assertEquals("the swipe moved the view", 5, back.displayOffset)
        // The content moved, not just a counter — a renderer draws what this
        // returns, so an offset that changed without the cells changing would
        // be a scrollbar sliding over a screen that never moves.
        assertNotEquals(rowText(live, 0), rowText(back, 0))
        assertTrue(
            "expected history on screen, got ${rowText(back, 0)}",
            rowText(back, 0).startsWith("history line "),
        )

        // Typing returns to the live screen, which is what `jumpToBottom` does.
        vt.scrollToBottom()
        assertEquals(0, vt.snapshot()!!.displayOffset)
        assertEquals("screen line 0", rowText(vt.snapshot()!!, 0))
        vt.free()
    }

    @Test
    fun theHostsScrollbackArrivesBase64AndIsReadyToFeed() {
        // What actually crosses the wire, decoded the way `TerminalSession`
        // decodes it. The host has already repaired the line feeds and appended
        // the colour reset, so nothing here repairs anything — that is the
        // point of the field.
        val encoded = Base64.encodeToString(scrollback, Base64.NO_WRAP)
        val decoded = Base64.decode(encoded, Base64.DEFAULT)
        assertTrue("the round trip must be lossless", decoded.contentEquals(scrollback))

        val vt = VtCore(80, 24)
        vt.feed(CapturedPane.feed(decoded, capture))
        assertTrue(vt.snapshot()!!.historySize > 0)
        vt.free()
    }

    @Test
    fun aShortCaptureStillLandsAtTheTopOfTheScreen() {
        // Why the clear is not decoration. tmux trims a capture's trailing
        // blank lines, so a quiet pane answers with two lines, not twenty-four
        // — and fed straight after the history those two lines land wherever
        // the history left the cursor, at the BOTTOM of a screen still showing
        // history. Measured against `crates/vt` with 40 history lines and a
        // two-line capture: without the clear the top row reads "history line
        // 18" and the core reports historySize = 18; with it the top row is
        // "screen line 0" and historySize is 40.
        val history = (0 until 40).joinToString("") { "history line $it\r\n" }.toByteArray()
        val short = "screen line 0\nscreen line 1\n".toByteArray()

        val vt = VtCore(80, 24)
        vt.feed(CapturedPane.feed(history, short))
        val live = vt.snapshot()!!
        assertEquals("screen line 0", rowText(live, 0))
        assertEquals("screen line 1", rowText(live, 1))
        assertEquals("", rowText(live, 2))
        assertEquals(40, live.historySize)
        vt.free()
    }

    private fun rowText(grid: TerminalGrid, row: Int): String = buildString {
        for (column in 0 until grid.columns) {
            val scalar = grid.character(row, column)
            append(if (scalar == 0) ' ' else scalar.toChar())
        }
    }.trimEnd()
}
