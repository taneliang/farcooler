package com.farcooler.ui

import com.farcooler.model.Trouble
import com.farcooler.net.AgentPhase
import com.farcooler.net.Waited
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What an empty agent pane says, in each of the states it can honestly be in.
 *
 * The screen had one bit — is there a connection error — and two answers, so it
 * went invitation → red failure → transcript and was wrong at the first two.
 * Pinning the copy per state is the only way that stays fixed: there is no
 * emulator here, and every one of these screens needs a runner whose shim is
 * slow or whose link is down to be looked at.
 */
class AgentEmptyStateTest {

    private val dropped = Trouble("The connection to this runner dropped. Reconnecting…")
    private val timedOut = Trouble(
        "The request that reads it didn’t finish.",
        "ssh: connect to host runner port 22: Operation timed out",
    )

    private val everyState = listOf(
        AgentPhase.Opening,
        AgentPhase.Starting(Waited.A_MOMENT),
        AgentPhase.Starting(Waited.A_WHILE),
        AgentPhase.Live,
        AgentPhase.Failing(dropped, Waited.A_MOMENT),
        AgentPhase.Failing(dropped, Waited.A_WHILE),
        AgentPhase.Failing(timedOut, Waited.TOO_LONG),
    )

    /**
     * The report itself: only a real failure that has been failing for thirty
     * seconds is drawn as one.
     */
    @Test
    fun `only a long failure gets the alarm`() {
        for (phase in everyState) {
            val alarming = agentEmptyState(phase).mark == AgentEmptyState.Mark.ALARM
            assertEquals(
                phase.toString(),
                phase == AgentPhase.Failing(timedOut, Waited.TOO_LONG),
                alarming,
            )
        }
    }

    /** Its converse: nothing that is still starting up says it could not load. */
    @Test
    fun `no waiting state wears a dead session's sentence`() {
        val stillTrying = everyState.filter { it != AgentPhase.Failing(timedOut, Waited.TOO_LONG) }
        for (phase in stillTrying) {
            val state = agentEmptyState(phase)
            for (word in listOf("could not", "couldn’t", "failed", "unable")) {
                assertFalse(
                    "$phase says “${state.title}”",
                    state.title.lowercase().contains(word),
                )
            }
        }
    }

    /**
     * The invitation appears in the one state it was ever true for.
     *
     * `AgentSupervisor::send` drops a prompt when no shim is connected, so
     * "Say something to begin." over a pane with no session is an invitation to
     * do something that silently does nothing.
     */
    @Test
    fun `only a live session invites a message`() {
        for (phase in everyState) {
            val invites = agentEmptyState(phase).title.contains("Say something")
            assertEquals(phase.toString(), phase == AgentPhase.Live, invites)
        }
    }

    /**
     * And no state advises sending one either — that advice is the same wrong
     * promise in a longer form.
     */
    @Test
    fun `no state tells anybody to send a message to start an agent`() {
        for (phase in everyState.filter { it != AgentPhase.Live }) {
            val state = agentEmptyState(phase)
            val prose = (state.title + " " + (state.message ?: "")).lowercase()
            for (word in listOf("send", "type", "message it", "ask it")) {
                assertFalse("$phase says “$prose”", prose.contains(word))
            }
        }
    }

    /** A pane with no agent says so, quietly, and names no cause. */
    @Test
    fun `a pane with no agent says so and blames nothing`() {
        val state = agentEmptyState(AgentPhase.Starting(Waited.A_WHILE))
        assertEquals("No agent on this pane yet", state.title)
        assertEquals(AgentEmptyState.Mark.CHAT, state.mark)
        // From this side of an SSH link the cause is unknowable, and a guess
        // sends somebody to change a setting that was never the problem.
        val prose = (state.title + " " + state.message).lowercase()
        for (word in listOf("shim", "daemon", "epoch", "ssh", "error", "scope")) {
            assertFalse(word, prose.contains(word))
        }
    }

    /**
     * Nothing spins forever.
     *
     * A spinner that never ends is its own bug, so past five seconds every
     * still-waiting state has swapped it for a mark and a sentence.
     */
    @Test
    fun `nothing spins past patience`() {
        for (waited in listOf(Waited.A_WHILE, Waited.TOO_LONG)) {
            assertFalse(
                waited.toString(),
                agentEmptyState(AgentPhase.Failing(dropped, waited)).mark ==
                    AgentEmptyState.Mark.SPINNER,
            )
        }
        assertFalse(
            agentEmptyState(AgentPhase.Starting(Waited.A_WHILE)).mark ==
                AgentEmptyState.Mark.SPINNER
        )
    }

    /**
     * The runner's own words survive, and only where there is a headline to put
     * them under.
     *
     * They are the only account anybody debugging an unreachable runner gets, so
     * nothing here rewrites or drops them — but they are never this app's
     * sentence, which is what the [Trouble] split is for.
     */
    @Test
    fun `the runner's own words reach the failure screen unchanged`() {
        val state = agentEmptyState(AgentPhase.Failing(timedOut, Waited.TOO_LONG))
        assertEquals("Could not load this session", state.title)
        assertEquals(timedOut.sentence, state.message)
        assertEquals(timedOut.transcript, state.transcript)
    }

    /** A first poll claims nothing about whether a session exists. */
    @Test
    fun `the first poll claims nothing either way`() {
        val state = agentEmptyState(AgentPhase.Opening)
        assertEquals(AgentEmptyState.Mark.SPINNER, state.mark)
        assertNull(state.message)
        assertNull(state.transcript)
    }

    /** Sentence case, per `cb13d31` — the acronym in the badge is the exception. */
    @Test
    fun `every headline is sentence case`() {
        for (phase in everyState) {
            val title = agentEmptyState(phase).title
            val rest = title.split(" ").drop(1).filter { it.isNotEmpty() }
            for (word in rest) {
                assertTrue(
                    "$phase capitalizes “$word” in “$title”",
                    !word[0].isUpperCase(),
                )
            }
        }
    }

    /**
     * The Mac's rule and the Mac's words: anything that is not `acp` is native.
     *
     * The three spellings are `BackendKind::as_str()`'s, from
     * `crates/agent-core/src/backend.rs` — not fixture spellings.
     */
    @Test
    fun `the adapter badge follows the Mac`() {
        assertEquals("ACP", adapterBadgeLabel("acp"))
        assertEquals("Native", adapterBadgeLabel("claude"))
        assertEquals("Native", adapterBadgeLabel("codex"))
    }

    /** A pane nobody has heard from names no protocol. */
    @Test
    fun `the adapter badge stays away until a session has said`() {
        assertNull(adapterBadgeLabel(null))
        assertNull(adapterBadgeLabel(""))
        assertNull(adapterBadgeDescription(null))
    }

    /** Two words on their own explain nothing to a screen reader. */
    @Test
    fun `the adapter badge explains itself out loud`() {
        assertTrue(adapterBadgeDescription("claude")!!.contains("claude"))
        assertTrue(adapterBadgeDescription("acp")!!.contains("Agent Client Protocol"))
    }
}
