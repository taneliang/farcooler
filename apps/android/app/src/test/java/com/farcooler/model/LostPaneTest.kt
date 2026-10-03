package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The page a terminal with no running pane opens to (ov-191).
 *
 * **The same table as AgentKit's `LostPaneTests`**, against the same words, so
 * the phone and the Mac can't come to say different things about one pane.
 */
class LostPaneTest {
    @Test
    fun theStatesWithNoPaneAreThisPage() {
        assertEquals(LostPane.Kind.LOST, LostPane.kind(StateKind.parse("LOST")))
        assertEquals(LostPane.Kind.LOST, LostPane.kind(StateKind.parse("lost")))
        assertEquals(LostPane.Kind.EXITED, LostPane.kind(StateKind.parse("exited")))
        assertEquals(LostPane.Kind.ERROR, LostPane.kind(StateKind.parse("error")))
        for (state in listOf("running", "starting", "unknown", "")) {
            assertNull(state, LostPane.kind(StateKind.parse(state)))
        }
    }

    @Test
    fun dismissIsOfferedOnlyWhereTheRunnerTakesIt() {
        assertEquals(listOf(LostPane.Action.RESTART, LostPane.Action.DISMISS), LostPane.actions(LostPane.Kind.LOST))
        assertEquals(listOf(LostPane.Action.RESTART), LostPane.actions(LostPane.Kind.EXITED))
        assertEquals(listOf(LostPane.Action.RESTART), LostPane.actions(LostPane.Kind.ERROR))
    }

    @Test
    fun lostSaysWhy() {
        val why = LostPane.explanation(LostPane.Kind.LOST)
        assertTrue(why.contains("closed outside Far Cooler"))
        assertTrue(why.contains("tmux was quit"))
        assertTrue(why.contains("the runner restarted"))
    }

    /** Restart without a recorded command: a shell says so. */
    @Test
    fun aShellSaysItsCommandWasNotRecorded() {
        for (preset in listOf("shell", "")) {
            val note = LostPane.restartNote(preset)
            assertTrue(preset, note.startsWith("Restart opens a new shell"))
            assertTrue(preset, note.contains("wasn’t recorded"))
        }
    }

    /** Restart with a recorded command: it's named. */
    @Test
    fun aRecordedPresetIsNamed() {
        assertTrue(LostPane.restartNote("claude:opus").contains("Claude Code"))
        assertTrue(LostPane.restartNote("codex").contains("Codex"))
        assertEquals("Restart runs sleepnomore again in this worktree.", LostPane.restartNote("sleepnomore"))
        for (preset in listOf("claude", "codex", "cursor", "sleepnomore")) {
            assertFalse(preset, LostPane.restartNote(preset).contains("wasn’t recorded"))
        }
    }

    @Test
    fun titlesAreTitleCase() {
        assertEquals("Terminal Lost", LostPane.title(LostPane.Kind.LOST))
        assertEquals("Terminal Didn’t Start", LostPane.title(LostPane.Kind.ERROR))
    }
}
