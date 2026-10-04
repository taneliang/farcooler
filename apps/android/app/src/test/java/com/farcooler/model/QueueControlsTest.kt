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

    /**
     * The row's state: an older runner dims all three actions and says why.
     * Mutation: `of` ignoring the gate. Red: enabled, no sentence.
     */
    @Test
    fun anOlderRunnersRowIsDimmedAndSaysWhy() {
        val row = QueueRowState.of(QueueControls.gate(runner("agent")), tappedEdit = false)
        assertFalse(row.actionsEnabled)
        assertEquals(QueueControls.OLDER_RUNNER_SENTENCE, row.sentence)
    }

    /** Mutation: `of` always dimmed. Red: no live actions with the capability. */
    @Test
    fun aCapableRunnersRowIsLiveAndSilent() {
        val row = QueueRowState.of(QueueControls.gate(runner("agent_queue")), tappedEdit = false)
        assertTrue(row.actionsEnabled)
        assertEquals(null, row.sentence)
    }

    /**
     * An edit open when the runner turns out to be gated closes rather than
     * stranding a dimmed Save. Mutation: `editing = tappedEdit`. Red: still open.
     */
    @Test
    fun anOpenEditClosesWhenTheRunnerIsGated() {
        assertTrue(QueueRowState.of(QueueControls.Available, tappedEdit = true).editing)
        val gated = QueueRowState.of(QueueControls.gate(runner("agent")), tappedEdit = true)
        assertFalse(gated.editing)
    }
}
