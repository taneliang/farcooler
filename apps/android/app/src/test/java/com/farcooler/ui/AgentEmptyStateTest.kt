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
        // Both failures at full width, not just the one that has host output to
        // show. A dropped link carries this app's sentence and nothing else,
        // which makes it the failure drawn most often and — until it was listed
        // here — the only `TOO_LONG` state no property below ever reached.
        AgentPhase.Failing(dropped, Waited.TOO_LONG),
        AgentPhase.Failing(timedOut, Waited.TOO_LONG),
    )

    /** Every state above that has been failing long enough to be called one. */
    private fun isLongFailure(phase: AgentPhase) =
        phase is AgentPhase.Failing && phase.waited == Waited.TOO_LONG

    /**
     * The report itself: only a real failure that has been failing for thirty
     * seconds is drawn as one.
     */
    @Test
    fun `only a long failure gets the alarm`() {
        for (phase in everyState) {
            val alarming = agentEmptyState(phase).mark == AgentEmptyState.Mark.ALARM
            assertEquals(phase.toString(), isLongFailure(phase), alarming)
        }
    }

    /** Its converse: nothing that is still starting up says it could not load. */
    @Test
    fun `no waiting state wears a dead session's sentence`() {
        val stillTrying = everyState.filter { !isLongFailure(it) }
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

    /**
     * The failure with nothing to quote, which is the ordinary one.
     *
     * A dropped link is [com.farcooler.net.Connection]'s own sentence and
     * nothing else: no command ran on the runner, so there is no host output to
     * put under the headline. That is what made this state the untested one —
     * the [everyState] list named the timed-out request as its only `TOO_LONG`
     * case, and the single question anybody asked of this one was whether it
     * had stopped spinning. A build that answered "Still trying", forever, to a
     * link that has been down for thirty seconds passed the whole file.
     *
     * A missing transcript is not a missing failure. The headline, the alarm
     * and the sentence are the same ones the timed-out request gets; only the
     * box underneath is absent, because there is nothing true to put in it.
     */
    @Test
    fun `a link down this long is a failure even with no host output to show`() {
        val state = agentEmptyState(AgentPhase.Failing(dropped, Waited.TOO_LONG))
        assertEquals(AgentEmptyState.Mark.ALARM, state.mark)
        assertEquals("Could not load this session", state.title)
        assertEquals(dropped.sentence, state.message)
        assertNull(state.transcript)
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

    // ---------------------------------------------------------------------
    // A chat with no agent in it.
    //
    // The runner has sent a stable machine word on `agentFailure` since the
    // shim stopped hanging on a failure, and this app decoded none of it: the
    // decoder ignores keys it does not know, so nothing broke and nothing
    // reported, and all three ways of failing to start an adapter drew
    // "Starting the agent…" forever. Run by `./gradlew testInstrumentedUnitTest`,
    // `.github/workflows/ci.yml:599`. The Apple half of these is
    // `AgentFailureTests` in `apps/shared/AgentKit`, word for word, because
    // Swift cannot import this.

    /**
     * The words, written out.
     *
     * The other half is `every_failure_has_a_stable_word` in
     * `farcooler-agent-core`, which pins the same four strings on the sending
     * side. Neither test can see the other, and that is exactly why both exist:
     * the wire between them is four literals and nothing else.
     */
    @Test
    fun `every word the runner sends has its own copy`() {
        assertEquals(AgentFailure.NO_ADAPTER, AgentFailure.of("no-adapter"))
        assertEquals(AgentFailure.NOT_AUTHENTICATED, AgentFailure.of("not-authenticated"))
        assertEquals(AgentFailure.ADAPTER_SILENT, AgentFailure.of("adapter-silent"))
        assertEquals(AgentFailure.ADAPTER_FAILED, AgentFailure.of("adapter-failed"))

        val titles = AgentFailure.entries.map { agentFailureState(it.word)!!.title }
        // Four failures, four different headlines. Each has a different fix — a
        // config entry, a login on the runner, waiting, and nobody knows — so a
        // shared sentence would be the endless spinner again with better
        // manners.
        assertEquals(AgentFailure.entries.size, titles.toSet().size)
    }

    /**
     * A verdict beats the ladder.
     *
     * [agentEmptyState] must not draw a spinner over a pane the runner has
     * already given up on. It cannot learn that from the phase: a pane whose
     * adapter failed holds no session, the daemon answers "no session", and
     * every state in the ladder reads that as a shim that is merely slow. That
     * is the reported bug — three failures, one spinner, forever.
     */
    @Test
    fun `a reported failure beats every state in the ladder`() {
        for (phase in everyState) {
            val state = agentEmptyState(phase, "not-authenticated")
            assertEquals(phase.toString(), "This agent needs you to sign in", state.title)
            assertEquals(phase.toString(), AgentEmptyState.Mark.ALARM, state.mark)
            assertFalse(phase.toString(), state.mark == AgentEmptyState.Mark.SPINNER)
        }
    }

    /**
     * A word this build has never heard of still says a pane failed.
     *
     * **The one place the phones part company with the Mac**, and deliberately.
     * The runner sends this field only to report that a pane gave up; reading a
     * fifth word as silence would put back the endless spinner this whole path
     * exists to end. It degrades to the generic failure — which is what
     * `adapter-failed` already means — and it still offers the way out.
     */
    @Test
    fun `a word from the future reads as a generic failure`() {
        val future = agentFailureState("from-the-future")
        assertEquals(agentFailureState("adapter-failed"), future)
        assertFalse(future!!.action.isNullOrEmpty())
        assertEquals(future, agentEmptyState(AgentPhase.Opening, "from-the-future"))
    }

    /**
     * A pane nobody has reported on is not a failed pane.
     *
     * The other direction, and it matters as much: a pane still coming up is
     * indistinguishable from a failed one from here, so calling it broken would
     * be the mirror image of the bug.
     */
    @Test
    fun `a pane with no word keeps the state it had`() {
        assertNull(agentFailureState(null))
        assertNull(agentFailureState(""))
        for (phase in everyState) {
            assertEquals(phase.toString(), agentEmptyState(phase), agentEmptyState(phase, null))
        }
    }

    /**
     * The word the runner sent must never be the words a person reads.
     *
     * The same assertion `runner_pipe.rs` makes about `TunnelError::code`.
     * Quoting the machine word back — "adapter-silent" on a screen — is the
     * failure this convention exists to prevent, and the easiest one to write
     * by accident when a sentence is being filled in quickly. The enum's own
     * case names are the other thing a hurried `when` puts on a screen.
     */
    @Test
    fun `no failure sentence quotes the machine word back`() {
        for (failure in AgentFailure.entries) {
            val state = agentFailureState(failure.word)!!
            val prose = state.title + " " + state.message + " " + state.action
            assertFalse("“$prose” quotes ${failure.word}", prose.contains(failure.word))
            assertFalse("“$prose” names ${failure.name}", prose.contains(failure.name))
        }
    }

    /**
     * Every failure is readable and every failure offers the terminal.
     *
     * The action is the point of the ruling: **the pane stays a chat**, and the
     * switch back is named rather than performed, because performing it
     * respawns the pane under whatever the reader was in the middle of typing.
     * A failure that forgot to offer it would leave them exactly as stuck as
     * the spinner did.
     */
    @Test
    fun `every failure is readable and offers the terminal`() {
        for (failure in AgentFailure.entries) {
            val state = agentFailureState(failure.word)!!
            assertTrue(failure.name, state.title.isNotEmpty())
            assertTrue(failure.name, !state.message.isNullOrEmpty())
            assertTrue(failure.name, state.title[0].isUpperCase())
            // Sentence case, and the pane header's own label for this action.
            assertEquals("Show the terminal", state.action)
        }
    }

    /** Sentence case here too — the rule above, over the states it cannot reach. */
    @Test
    fun `every failure headline is sentence case`() {
        for (failure in AgentFailure.entries) {
            val title = agentFailureState(failure.word)!!.title
            for (word in title.split(" ").drop(1).filter { it.isNotEmpty() }) {
                assertTrue("“$title” capitalizes “$word”", !word[0].isUpperCase())
            }
        }
    }
}
