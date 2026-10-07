package com.farcooler.model

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** A draft held behind a dialog (ov-385): the answer read, the pane's words, and nothing copied. */
class HeldDraftTest {
    private val json = Json { ignoreUnknownKeys = true }

    /** `{"held": …}` as the FFI writes it (`draft_hold_json`). */
    private val heldAnswer =
        """{"held":{"id":"0198f2c0-0000-7000-8000-00000000f385","state":"waiting","heldMs":1000,"expiresMs":1801000,"endedMs":0}}"""

    @Test
    fun aHeldAnswerIsReadAndAnyOtherIsAPaste() {
        val hold = DraftHold.held(json.decodeFromString<JsonObject>(heldAnswer))
        assertEquals(DraftHold("0198f2c0-0000-7000-8000-00000000f385", "waiting", 1_801_000), hold)
        assertNull(DraftHold.held(json.decodeFromString<JsonObject>("{}")))
    }

    @Test
    fun aFleetRowsHoldDecodesAndAnUnknownStateIsNeverWaiting() {
        val hold = json.decodeFromString<DraftHold>("""{"id":"h","state":"sent","heldMs":1,"expiresMs":2,"endedMs":3}""")
        assertEquals(DraftHold("h", "sent", 2), hold)
        assertEquals(HeldDraft.Status.EXPIRED, HeldDraft.status("h", DraftHold("h", "parked")))
    }

    @Test
    fun aHeldDraftIsNeitherCopiedNorNoticed() = runBlocking {
        val copied = mutableListOf<String>()
        val delivery = AskAboutTask.deliver(
            "ov-9", "Move it", isAgentPane = false,
            offer = {}, paste = { AskAboutTask.DraftResult.HELD }, copy = { copied += it },
        )
        assertEquals(AskAboutTask.Delivery.HELD, delivery)
        assertTrue(copied.isEmpty())
        assertNull(RulingActions.notice(RulingActions.Reversal.HELD, PlanRuling("r", "R-1", 1, "A", "B", "C")))
    }

    @Test
    fun thePaneSaysWaitingThenSentAndLetsGoOfAWithdrawnOne() {
        assertEquals(HeldDraft.Status.WAITING, HeldDraft.status("h", DraftHold("h", "waiting")))
        assertEquals("Waiting for the dialog to close", HeldDraft.title(HeldDraft.Status.WAITING))
        assertEquals("Sent", HeldDraft.title(HeldDraft.status("h", DraftHold("h", "sent"))))
        assertEquals(HeldDraft.Status.GONE, HeldDraft.status("h", DraftHold("h", "withdrawn")))
        assertEquals(HeldDraft.Status.GONE, HeldDraft.status("h", DraftHold("newer", "waiting")))
        assertEquals(HeldDraft.Status.LOST, HeldDraft.status("h", null))
        assertNull(HeldDraft.title(HeldDraft.Status.GONE))
    }
}
