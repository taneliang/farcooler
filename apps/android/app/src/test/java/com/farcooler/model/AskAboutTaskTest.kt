package com.farcooler.model

import com.farcooler.model.RunnerRefusal
import com.farcooler.ui.OrchestratorSeat
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AskAboutTaskTest {
    @Test
    fun theDraftNamesTheTaskAndEndsWhereTheyKeepTyping() {
        assertEquals(
            "About ov-241 (“Phones: Ask the Orchestrator”): ",
            AskAboutTask.draft("ov-241", "Phones: Ask the Orchestrator"),
        )
    }

    @Test
    fun aTaskTitleCannotDoAnythingWhereItLands() {
        // A CSI that would close a bracketed paste, an OSC, ^C, bidi and a newline.
        val hostile = "fix\u001B[201~ it\u001B]0;x\u0007 now\u0003‮!\nnext"
        assertEquals("fix it now! next", AskAboutTask.oneLine(hostile))
        assertTrue(!AskAboutTask.draft("ov-1", hostile).contains('\u001B'))
    }

    @Test
    fun aChatOrchestratorGetsTheDraftInItsComposerAndNothingIsPasted() = runBlocking {
        val offered = mutableListOf<String>()
        var pasted = 0
        val copied = mutableListOf<String>()
        val delivery = AskAboutTask.deliver(
            "ov-9", "Move it", isAgentPane = true,
            offer = { offered += it }, paste = { pasted++; AskAboutTask.DraftResult.PASTED }, copy = { copied += it },
        )
        assertEquals(AskAboutTask.Delivery.COMPOSER, delivery)
        assertEquals(listOf("About ov-9 (“Move it”): "), offered)
        assertEquals(0, pasted)
        assertTrue(copied.isEmpty())
    }

    @Test
    fun aTerminalOrchestratorIsPastedToAndOtherwiseTheReferenceIsCopied() = runBlocking {
        var offered = 0
        val asked = mutableListOf<String>()
        val copied = mutableListOf<String>()
        val pasted = AskAboutTask.deliver(
            "ov-9", "Move it", isAgentPane = false,
            offer = { offered++ }, paste = { asked += it; AskAboutTask.DraftResult.PASTED }, copy = { copied += it },
        )
        assertEquals(AskAboutTask.Delivery.PASTED, pasted)
        assertEquals(listOf("About ov-9 (“Move it”): "), asked)
        assertTrue(copied.isEmpty())
        assertEquals(0, offered)

        // The runner refused: nothing is typed, and the reference is on the
        // clipboard without the trailing space.
        val refused = AskAboutTask.deliver(
            "ov-9", "Move it", isAgentPane = false,
            offer = { offered++ }, paste = { AskAboutTask.DraftResult.DECLINED }, copy = { copied += it },
        )
        assertEquals(AskAboutTask.Delivery.COPIED, refused)
        assertEquals(listOf("About ov-9 (“Move it”):"), copied)
        assertEquals(
            "Copied a reference to ov-9. Paste it into the orchestrator.",
            AskAboutTask.copiedNotice("ov-9"),
        )
    }

    @Test
    fun aPasteThatMayHaveLandedIsNotAlsoCopied() = runBlocking {
        val copied = mutableListOf<String>()
        val delivery = AskAboutTask.deliver(
            "ov-9", "Move it", isAgentPane = false,
            offer = {}, paste = { AskAboutTask.DraftResult.UNKNOWN }, copy = { copied += it },
        )
        assertEquals(AskAboutTask.Delivery.MAYBE_PASTED, delivery)
        assertTrue("copied and pasted both", copied.isEmpty())
        val slow = com.farcooler.core.CoreException("slow", RunnerRefusal.TIMED_OUT_WORD)
        assertEquals(AskAboutTask.DraftResult.UNKNOWN, AskAboutTask.DraftResult.of(slow))
        assertEquals(
            AskAboutTask.DraftResult.UNKNOWN,
            AskAboutTask.DraftResult.of(com.farcooler.core.DisconnectedException("gone")),
        )
        assertEquals(
            AskAboutTask.DraftResult.DECLINED,
            AskAboutTask.DraftResult.of(com.farcooler.core.DisconnectedException("not connected", notSent = true)),
        )
        assertEquals(
            AskAboutTask.DraftResult.DECLINED,
            AskAboutTask.DraftResult.of(com.farcooler.core.CoreException("no", "agent-not-connected")),
        )
    }

    private fun workspace(id: String, implicit: Boolean = false, orchestrator: String? = null) =
        WorkspaceSummary(
            id = id, name = "Billing", taskPrefix = "bil", isMain = false, ordinal = 1,
            orchestrator = orchestrator, isImplicit = implicit,
        )

    private fun lane(vararg terminals: Terminal) = Worktree(id = "w1", terminals = terminals.toList())

    @Test
    fun theSeatIsALiveOrchestratorOfThisWorkspaceAndNothingElse() {
        val live = Terminal(id = "o", state = "running", role = "orchestrator", workspace = "ws")
        val found = AskAboutTask.seat("ws", listOf(lane(live)), listOf(workspace("ws")))
        assertEquals("o", found?.terminal?.id)
        assertEquals("w1", found?.worktreeId)

        // Named by the runner when no pane says so itself.
        val plain = Terminal(id = "p", state = "running")
        assertEquals(
            "p", AskAboutTask.seat("ws", listOf(lane(plain)), listOf(workspace("ws", orchestrator = "p")))?.terminal?.id)

        // A dead one, another workspace's, an implicit workspace and an unlisted one are none.
        val dead = live.copy(state = "exited")
        assertNull(AskAboutTask.seat("ws", listOf(lane(dead)), listOf(workspace("ws"))))
        assertNull(AskAboutTask.seat("other", listOf(lane(live)), listOf(workspace("other"))))
        assertNull(AskAboutTask.seat("ws", listOf(lane(live)), listOf(workspace("ws", implicit = true))))
        assertNull(AskAboutTask.seat("ws", listOf(lane(live)), null))
        assertTrue(AskAboutTask.seat("ws", emptyList(), listOf(workspace("ws"))) == null)
        assertTrue(OrchestratorSeat.of("o", listOf(lane(live)), null, 0, false) is OrchestratorSeat.Live)
    }
}
