package com.farcooler.net

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The screen response, and the field that broke it.
 *
 * The first build on real hardware showed "Could not load" on roughly every
 * second keystroke. The cause was `revision` declared as a `Long`: the host
 * computes it as an FNV-1a hash over the capture and the cursor, so it uses the
 * whole `u64` range, and a literal above `Long.MAX_VALUE` does not truncate —
 * the parser refuses it and the entire response is lost.
 *
 * The exact bytes below are from that failure, so the test fails on the version
 * of the code that produced it.
 */
class ScreenResponseTest {
    private val json = Json { ignoreUnknownKeys = true }

    private fun decode(text: String) = json.decodeFromString(ScreenResponse.serializer(), text)

    @Test
    fun aRevisionAboveLongMaxValueDecodes() {
        // 13074097809131186115 > 9223372036854775807. Captured from a Pixel 10
        // running a real terminal.
        val response = decode(
            """{"contents":"","columns":80,"rows":17,"cursorColumn":0,"cursorRow":0,""" +
                """"revision":13074097809131186115,"unchanged":false}"""
        )
        assertEquals(13074097809131186115uL, response.revision)
        assertEquals(17, response.rows)
        assertFalse(response.unchanged)
    }

    @Test
    fun theWholeUnsignedRangeSurvives() {
        // The host guarantees only that the hash is never zero — every other
        // value is reachable, including the very top of the range.
        assertEquals(ULong.MAX_VALUE, decode("""{"revision":18446744073709551615}""").revision)
        assertEquals(1uL, decode("""{"revision":1}""").revision)
    }

    @Test
    fun aMissingRevisionIsZeroWhichMeansNothingIsKnown() {
        // Zero is the wire's "I have nothing", and the host is careful never to
        // produce it as a real hash — so defaulting to it is safe and asks for
        // a full capture rather than accidentally claiming a screen is
        // unchanged.
        assertEquals(0uL, decode("""{"columns":80}""").revision)
    }

    @Test
    fun aKnownRevisionGoesBackAsANumberRatherThanAString() {
        // The host reads this with `as_u64()`, which answers `None` for a
        // string and silently falls back to zero — so quoting it would not
        // fail, it would just ask for a full capture every time and quietly
        // undo the optimisation the field exists for.
        val args = Connection.args("terminal" to "t", "knownRevision" to 13074097809131186115uL)
        val encoded = args.toString()
        assertTrue(
            "expected an unquoted number, got $encoded",
            encoded.contains("\"knownRevision\":13074097809131186115"),
        )
        // And the terminal id beside it is still a string.
        assertEquals("t", args["terminal"]?.jsonPrimitive?.contentOrNull)
    }

    @Test
    fun anUnchangedAnswerCarriesNoCapture() {
        // What the host sends when the revision it was handed still matches:
        // a hundred bytes rather than a whole screen.
        val response = decode("""{"revision":13074097809131186115,"unchanged":true}""")
        assertTrue(response.unchanged)
        assertEquals("", response.contents)
    }
}
