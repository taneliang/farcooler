package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** ov-171. The same cases as AgentKit's `QueueControlsTests`. */
class QueueControlsTest {
    private fun runner(vararg caps: String) =
        DaemonBuild("v", true, "linux", capabilities = caps.toSet())

    /** Mutation: `gate` always Available. Red: no sentence. */
    @Test
    fun aRunnerWithoutTheCapabilityGetsTheSentence() {
        val gate = QueueControls.gate(runner("agent", "tasks"))
        assertEquals(QueueControls.Unavailable(QueueControls.OLDER_RUNNER_SENTENCE), gate)
        assertFalse(gate.isAvailable)
    }

    @Test
    fun aRunnerFromBeforeCapabilitiesIsGatedToo() {
        assertFalse(QueueControls.gate(runner()).isAvailable)
    }

    /** Mutation: `gate` always Unavailable. Red: controls never shown. */
    @Test
    fun aRunnerWithTheCapabilityShowsTheControls() {
        assertTrue(QueueControls.gate(runner("agent", "agent_queue")).isAvailable)
    }

    @Test
    fun aRunnerNotYetHeardFromIsNotGated() {
        assertTrue(QueueControls.gate(null).isAvailable)
    }

    @Test
    fun theSentenceReadsPlainly() {
        assertEquals(
            "This runner can’t change queued messages. Update it to edit or cancel them.",
            QueueControls.OLDER_RUNNER_SENTENCE,
        )
    }

    /** Mutation: `refusal` returning only the step. Red: the reason is missing. */
    @Test
    fun aRefusalIsSaidAndKeepsItsStep() {
        val said = QueueControls.refusal(QueueControls.Action.CANCEL, "agent-stopped", "raw")
        assertTrue(said.startsWith("Couldn’t take that message back."))
        assertTrue(said.contains("The agent stopped."))
        assertFalse(said.contains("raw"))
    }
}
