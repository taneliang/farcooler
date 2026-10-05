package com.farcooler.model

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The owner's actions on a ruling (ov-333), word for word AgentKit's
 * `RulingActions`: Keep is local, Reverse sends the recorded reversal to the
 * orchestrator, Discuss quotes the ruling unsent.
 */
class RulingActionsTest {
    private val ruling = PlanRuling(
        "r", "R-12", 12, "The inbox is amber.", "One attention color.", "One token; every surface follows.",
    )

    @Test
    fun `reverse sends the recorded reversal to a chat orchestrator and leaves the composer alone`() = runBlocking {
        val sent = mutableListOf<String>()
        var pasted = 0
        var copied = 0
        val outcome = RulingActions.reverse(
            ruling, isAgentPane = true,
            send = { sent += it; true },
            paste = { pasted++; AskAboutTask.DraftResult.PASTED },
            copy = { copied++ },
        )
        assertEquals(RulingActions.Reversal.SENT, outcome)
        assertEquals(1, sent.size)
        assertTrue(sent[0], "One token; every surface follows." in sent[0])
        assertTrue("ruling R-12" in sent[0])
        assertTrue("`plan ruling reverse R-12 --sha <commit>`" in sent[0])
        assertEquals(0, pasted + copied)
    }

    @Test
    fun `a refused send is a failure the owner is told about`() = runBlocking {
        val outcome = RulingActions.reverse(
            ruling, true, send = { false }, paste = { AskAboutTask.DraftResult.PASTED }, copy = {},
        )
        assertEquals(RulingActions.Reversal.FAILED, outcome)
        assertEquals("Couldn’t reach the orchestrator. Try again.", RulingActions.notice(outcome, ruling))
    }

    @Test
    fun `a terminal orchestrator gets it typed with no Enter, or on the clipboard, and is never sent a message`() = runBlocking {
        var sent = 0
        val copied = mutableListOf<String>()
        fun reverse(result: AskAboutTask.DraftResult) = runBlocking {
            RulingActions.reverse(ruling, false, send = { sent++; true }, paste = { result }, copy = { copied += it })
        }
        assertEquals(RulingActions.Reversal.DRAFTED, reverse(AskAboutTask.DraftResult.PASTED))
        assertEquals(RulingActions.Reversal.MAYBE_DRAFTED, reverse(AskAboutTask.DraftResult.UNKNOWN))
        assertEquals(RulingActions.Reversal.COPIED, reverse(AskAboutTask.DraftResult.DECLINED))
        assertEquals(0, sent)
        assertEquals(1, copied.size)
        assertTrue("One token" in copied[0])
        assertEquals("Typed, not sent. Check the orchestrator’s input for the request.", RulingActions.notice(RulingActions.Reversal.MAYBE_DRAFTED, ruling))
    }

    @Test
    fun `discuss quotes the ruling into the composer, unsent`() = runBlocking {
        val offered = mutableListOf<String>()
        val delivery = RulingActions.discuss(
            ruling, true, offer = { offered += it }, paste = { AskAboutTask.DraftResult.PASTED }, copy = {},
        )
        assertEquals(AskAboutTask.Delivery.COMPOSER, delivery)
        assertEquals(listOf("About ruling R-12 (“The inbox is amber.”): "), offered)
    }

    @Test
    fun `a ruling's words are made safe before they reach a composer or a pane`() {
        val hostile = PlanRuling("r", "R-3", 3, "Blue\u001B[201~ now\nsecond line", "", "Do it\r\n\u001B]0;title\u0007fast")
        assertEquals("About ruling R-3 (“Blue now second line”): ", RulingActions.discussDraft(hostile))
        val text = RulingActions.reverseRequest(hostile)
        assertFalse('\u001B' in text || '\n' in text || '\r' in text)
        assertTrue("Do it fast" in text)
    }

    @Test
    fun `states read in the owner's words, and a reversal names its commit`() {
        assertEquals("Open", RulingWords.state(RulingState.STANDING))
        assertEquals("Kept", RulingWords.state(RulingState.CONFIRMED))
        val reversed = ruling.copy(state = RulingState.REVERSED)
        assertEquals("Reversed", RulingWords.settled(reversed))
        assertEquals("Reversed in 6e7e5618", RulingWords.settled(reversed.copy(reversedSha = "6e7e5618")))
    }

    @Test
    fun `open and past split the list, and the past leads with the newest settled`() {
        val plan = Plan(
            rulings = listOf(
                ruling,
                ruling.copy(id = "a", short = "R-1", number = 1, state = RulingState.CONFIRMED, settledAt = 10),
                ruling.copy(id = "b", short = "R-2", number = 2, state = RulingState.REVERSED, settledAt = 20),
            ),
        )
        assertEquals(listOf("R-12"), plan.openRulings.map { it.short })
        assertEquals(listOf("R-2", "R-1"), plan.pastRulings.map { it.short })
        assertEquals("board_ruling_actions", Capability.BOARD_RULING_ACTIONS.wire)
    }

    @Test
    fun `reverse confirms by naming the ruling and its reversal`() {
        assertEquals("Reverse R-12?", RulingActions.confirmTitle(ruling))
        val message = RulingActions.confirmMessage(ruling)
        assertTrue("The inbox is amber." in message)
        assertTrue("One token; every surface follows." in message)
    }
}
