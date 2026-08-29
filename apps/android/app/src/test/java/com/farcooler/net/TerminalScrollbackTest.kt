package com.farcooler.net

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The escape character, spelled rather than pasted, so this file stays text. */
private const val ESC = "\u001B"

/**
 * The half of the scrollback fix that does not need a device.
 *
 * Every enrolled runner answers a poll with `capture-pane -e -p` — the visible
 * screen and no history — because `crates/fence` writes a forced SSH command
 * and the stream that would have carried scrollback never delivers a byte. Fed
 * into an emulator exactly as tall as the screen, that leaves nothing above the
 * top row, and a swipe on this phone moves nothing. The fix is to ask the host
 * for the history and feed it above the screen.
 *
 * What is testable here is the shape of the ask and the order of the bytes. The
 * number that proves it worked — the core's own history size — needs the real
 * emulator, so it is asserted in `NativeBridgeTest` on a device instead.
 */
class TerminalScrollbackTest {
    private val json = Json { ignoreUnknownKeys = true }

    private fun decode(text: String) = json.decodeFromString(ScreenResponse.serializer(), text)

    @Test
    fun theScreenAskCarriesHowMuchScrollbackToSend() {
        // The host reads this with `as_u64()` (`crates/client/src/ffi.rs`),
        // which answers `None` for a string and silently falls back to zero —
        // so quoting it would not fail, it would just ask for no history at all
        // and quietly undo the whole fix.
        val encoded = TerminalSession.screenArgs("t").toString()
        assertTrue(
            "expected an unquoted number, got $encoded",
            encoded.contains("\"historyLines\":${TerminalSession.HISTORY_LINES}"),
        )
        assertTrue("the ask must still name the pane", encoded.contains("\"terminal\":\"t\""))
    }

    @Test
    fun aResponsesScrollbackIsRead() {
        // "hi\r\n" base64'd. The host sends bytes that are already ready to
        // feed: line feeds repaired, colour reset appended.
        val response = decode("""{"columns":80,"rows":24,"history":"aGkNCg=="}""")
        assertEquals("aGkNCg==", response.history)
    }

    @Test
    fun aRunnerThatKnowsNothingOfScrollbackIsNotAFailure() {
        // A daemon too old to know the field omits it, and a pane with no
        // history sends it empty. Both mean "no scrollback", and neither is an
        // error — this app has to keep showing terminals against either.
        assertNull(decode("""{"columns":80,"rows":24}""").history)
        assertEquals("", decode("""{"history":""}""").history)
    }

    @Test
    fun theScrollbackGoesInFirstAndAClearPushesItUp() {
        val history = "old line\r\n".toByteArray()
        val screen = "prompt\n".toByteArray()
        val fed = String(CapturedPane.feed(history, screen))

        // Order, and the clear between them. Erasing the display is what pushes
        // the history lines up into scrollback rather than discarding them, so
        // a clear that landed anywhere else would be a pane that still does not
        // scroll.
        assertEquals("old line\r\n$ESC[H$ESC[2Jprompt", fed)
        assertTrue(fed.indexOf("old line") < fed.indexOf("$ESC[2J"))
        assertTrue(fed.indexOf("$ESC[2J") < fed.indexOf("prompt"))
    }

    @Test
    fun noScrollbackMeansNoClearAtAll() {
        // The common case — a pane that has printed less than a screenful, or
        // one on the alternate screen, which has no history of its own. A clear
        // with nothing above it would erase nothing and cost bytes to say so.
        val fed = String(CapturedPane.feed(ByteArray(0), "prompt\n".toByteArray()))
        assertEquals("prompt", fed)
    }

    @Test
    fun theScreenIsStillRepairedWithScrollbackAboveIt() {
        // The bare line feeds in the SCREEN are still this app's to repair —
        // the host only repaired the history it assembled. Without this every
        // line starts where the previous one ended and the screen arrives as a
        // staircase.
        val fed = String(CapturedPane.feed("h\r\n".toByteArray(), "one\ntwo\n".toByteArray()))
        assertTrue("expected the screen's line feeds repaired, got $fed", fed.endsWith("one\r\ntwo"))
    }
}
