package com.farcooler.core

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/** The escape character, spelled rather than pasted, so this file stays text. */
private const val ESC = ""

/**
 * The JNI bridge, on a real device.
 *
 * These cannot run as JVM unit tests: there is no `libfarcooler_jni.so` on a
 * desktop JVM's library path, and mocking it would test the mock. They exist
 * because everything above this boundary is written as though the native side
 * works, and the failure when it does not is an `UnsatisfiedLinkError` on the
 * first screen someone opens rather than anything a compiler could have caught.
 *
 * Deliberately narrow: they prove the bridge carries values in both directions
 * and that the cores behave through it. What the emulator parses is what the
 * Rust suites already cover — this is about the join, not the emulator.
 */
@RunWith(AndroidJUnit4::class)
class NativeBridgeTest {

    @Test
    fun theSharedObjectLoads() {
        // If this fails nothing else in the app can work, so it is asserted
        // first and on its own rather than being inferred from a later failure.
        assertTrue("libfarcooler_jni.so did not load", coreIsAvailable)
    }

    @Test
    fun bytesFedToTheEmulatorComeBackAsCells() {
        val vt = VtCore(40, 6)
        vt.feed("hi".toByteArray())
        val grid = vt.snapshot()
        assertNotNull(grid)
        assertEquals(40, grid!!.columns)
        assertEquals(6, grid.rows)
        assertEquals('h'.code, grid.character(0, 0))
        assertEquals('i'.code, grid.character(0, 1))
        // The cursor comes back too, and it is where the text left it.
        assertEquals(2, grid.cursorColumn)
        vt.free()
    }

    @Test
    fun aGridTooSmallToBeATerminalIsClampedRatherThanHonoured() {
        // The core floors a terminal at 20×5 (`crates/vt/src/lib.rs`, `clamp`),
        // and the header states it. Asserted here because the clamp has to
        // survive the JNI narrowing to `u16` on the way in — a renderer that
        // believed it got the four rows it asked for would index one row past
        // the end of every snapshot.
        val vt = VtCore(3, 4)
        val grid = vt.snapshot()!!
        assertEquals(20, grid.columns)
        assertEquals(5, grid.rows)
        vt.free()
    }

    @Test
    fun aNegativeDimensionSaturatesRatherThanWrapping() {
        // `clamp_u16` in the shim saturates, so a bad layout pass produces a
        // strange-looking terminal instead of a 1-column one from a value of
        // 65,537 — or an enormous allocation from a negative one.
        val vt = VtCore(-1, -1)
        val grid = vt.snapshot()!!
        assertEquals(20, grid.columns)
        assertEquals(5, grid.rows)
        vt.free()
    }

    @Test
    fun coloursArriveAlreadyResolved() {
        // The core resolves the named and 256-colour palettes so a renderer
        // never has to. Which red is the core's business; that it changed and
        // that it is drawable is this test's.
        val vt = VtCore(10, 2)
        vt.feed("$ESC[31mx".toByteArray())
        val grid = vt.snapshot()!!
        val foreground = grid.foreground(0, 0)
        assertEquals("must be opaque", 0xFF, (foreground ushr 24) and 0xFF)
        assertTrue(
            "red should dominate",
            ((foreground shr 16) and 0xFF) > (foreground and 0xFF),
        )
        vt.free()
    }

    @Test
    fun aKeystrokeIsEncodedByTheCoreRatherThanGuessedAt() {
        val vt = VtCore(10, 2)
        // Ctrl-C is `c` with the control modifier, not a key of its own — the
        // core is what knows that, which is the whole reason encoding crosses
        // the boundary instead of being done in Kotlin.
        assertTrue(vt.encodeKey('c'.code, Vt.MOD_CTRL).contentEquals(byteArrayOf(0x03)))
        assertTrue(vt.encodeKey(Vt.KEY_ENTER, 0).isNotEmpty())
        vt.free()
    }

    @Test
    fun aPasteIsBracketedOnlyWhenTheProgramAskedForIt() {
        val vt = VtCore(10, 2)
        assertTrue(vt.encodePaste("ls").contentEquals("ls".toByteArray()))

        // `2004h` is the program saying it wants bracketed paste. Without it an
        // editor auto-indents every pasted line and a shell runs each newline
        // as a command.
        vt.feed("$ESC[?2004h".toByteArray())
        val bracketed = String(vt.encodePaste("ls"))
        assertTrue("expected bracketing, got $bracketed", bracketed.contains("[200~"))
        vt.free()
    }

    @Test
    fun aWheelEventIsRefusedUntilTheProgramWantsOne() {
        val vt = VtCore(10, 2)
        // Null means "handle it locally", which is what makes this device's own
        // scrollback work outside a full-screen program.
        assertNull(vt.encodeMouse(Vt.MOUSE_WHEEL_UP, Vt.MOUSE_PRESS, 0, 0, 0))

        vt.feed("$ESC[?1000h".toByteArray())
        assertNotNull(vt.encodeMouse(Vt.MOUSE_WHEEL_UP, Vt.MOUSE_PRESS, 0, 0, 0))
        vt.free()
    }

    @Test
    fun scrollbackIsTheClientsOwnViewAndSendsNothing() {
        val vt = VtCore(40, 6)
        // Far more lines than the screen holds, so there is real history to
        // scroll into. Feeding exactly a screenful puts nothing above it, and
        // the clamp then correctly refuses to move — which looks like a broken
        // scrollback and is not one.
        vt.feed((0 until 30).joinToString("") { "line$it\r\n" }.toByteArray())

        val live = vt.snapshot()!!
        assertEquals(0, live.displayOffset)
        assertTrue("history should have accumulated", live.historySize > 0)
        val liveTop = rowText(live, 0)

        vt.scroll(10)
        val back = vt.snapshot()!!
        assertEquals(10, back.displayOffset)
        // The content moved, not just a counter: a renderer draws what this
        // returns, so an offset that changed without the cells changing would
        // be a scrollbar that slides over a screen that never moves.
        assertNotEquals(liveTop, rowText(back, 0))

        vt.scrollToBottom()
        val returned = vt.snapshot()!!
        assertEquals(0, returned.displayOffset)
        assertEquals(liveTop, rowText(returned, 0))

        // And none of it produced a byte for the program: scrollback is this
        // device's own view, and the host is never told about it.
        assertEquals(0, vt.takeWrites().size)
        vt.free()
    }

    private fun rowText(grid: TerminalGrid, row: Int): String = buildString {
        for (column in 0 until grid.columns) {
            val scalar = grid.character(row, column)
            append(if (scalar == 0) ' ' else scalar.toChar())
        }
    }.trimEnd()

    @Test
    fun aGeneratedKeyIsAUsableOpenSshPair() {
        val pair = ClientCore.generateKey("farcooler-instrumented-test")
        assertNotNull(pair)
        val (private, public) = pair!!
        assertTrue(private.startsWith("-----BEGIN OPENSSH PRIVATE KEY-----"))
        assertTrue(public.startsWith("ssh-ed25519 "))
        // The comment identifies the device, which is what makes revoking one
        // possible without guessing which line in authorized_keys is which.
        assertTrue(public.trim().endsWith("farcooler-instrumented-test"))

        // And the public half derives from the private one rather than being
        // stored beside it — two sources for one fact diverge, and the symptom
        // is a runner rejecting a correct-looking key.
        val derived = ClientCore.publicKey(private)
        assertNotNull(derived)
        assertEquals(public.split(" ").take(2), derived!!.split(" ").take(2))
    }

    @Test
    fun aFreedHandleIsSurvivable() {
        // A UI lifecycle bug must not take the process down in native code,
        // where the crash leaves no Kotlin stack trace worth reading.
        val vt = VtCore(5, 2)
        vt.free()
        vt.free()
        vt.feed("x".toByteArray())
        assertNull(vt.snapshot())
        assertTrue(vt.encodeKey('a'.code, 0).isEmpty())
    }
}
