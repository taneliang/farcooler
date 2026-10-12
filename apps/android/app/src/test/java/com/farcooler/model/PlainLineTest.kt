package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Test

/** An orchestrator's reply on the peek reads as words, never as Markdown (ov-495). */
class PlainLineTest {
    private val said =
        "Four asks, in order: - **PDF export** (ha-12): queued behind ha-10. - **Charge retries**: building now in `charge-retry`."

    @Test
    fun aReplyThatOpensWithAListLosesItsMarkup() {
        assertEquals(
            "Four asks, in order: PDF export (ha-12): queued behind ha-10. Charge retries: building now in charge-retry.",
            PlainLine.of(said),
        )
    }

    @Test
    fun linesKeepOnlyTheFirstWordsAndProseSurvives() {
        assertEquals("Four asks, in order:", PlainLine.of("Four asks, in order:\n\n- **PDF export**\n- more"))
        assertEquals("PDF export (ha-12)", PlainLine.of("\n- **PDF export** (ha-12)\n- more"))
        assertEquals("Ship the docs now", PlainLine.of("1. Ship [the docs](https://x.example/a_b) _now_"))
        val prose = "Done - tests pass, 2 * 3 is 6, snake_case_name stays."
        assertEquals(prose, PlainLine.of(prose))
    }

    @Test
    fun theStripsLineIsPlain() {
        val terminal = Terminal(id = "o", state = "running", activity = "idle", role = "orchestrator", said = said)
        assertEquals(PlainLine.of(said), PlanStrip.line(terminal, PlanStrip.Orchestrator.IDLE))
    }
}
