package com.farcooler.model

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import com.farcooler.model.AgentConversation.SendFailure
import com.farcooler.model.AgentConversation.SendIssue
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The phone's rules for a claude pane's conversation view (AgentKit's `AgentConversationTests`). */
class AgentConversationTest {
    private fun build(vararg caps: Capability, scope: String = "control") =
        DaemonBuild("1", true, "linux", caps.map { it.wire }.toSet(), grantedScope = scope)

    private val serving = build(Capability.AGENT_ROWS, Capability.AGENT_COMPOSE)
    private fun claude(state: String = "running", paneMode: String? = null, preset: String = "claude") =
        Terminal(id = "t1", preset = preset, state = state, paneMode = paneMode)

    @Test
    fun `a runner serves the view only with rows and compose both`() {
        assertTrue(AgentConversation.served(serving))
        // Rows from before compose: the terminal, not a view whose every send fails.
        assertFalse(AgentConversation.served(build(Capability.AGENT_ROWS)))
        assertFalse(AgentConversation.served(build(Capability.AGENT_COMPOSE)))
        assertFalse(AgentConversation.served(build(Capability.TERMINALS)))
        assertFalse(AgentConversation.served(null))
    }

    @Test
    fun `the view is offered on a running claude in a terminal and nowhere else`() {
        assertTrue(AgentConversation.offered(serving, null, claude()))
        assertTrue(AgentConversation.offered(serving, null, claude(state = "starting", preset = "claude-yolo")))
        // Not claude, not a terminal, not running, or no terminal at all.
        // Codex only where the runner projects it and composes into it (ov-416).
        assertFalse(AgentConversation.offered(serving, null, claude(preset = "codex")))
        val codexServing = build(Capability.AGENT_ROWS, Capability.AGENT_COMPOSE, Capability.CODEX_VIEW)
        assertTrue(AgentConversation.offered(codexServing, null, claude(preset = "codex")))
        assertTrue(AgentConversation.offered(null, codexServing, claude(preset = "codex:gpt-5")))
        assertFalse(AgentConversation.offered(codexServing, null, claude(preset = "codex", paneMode = "agent")))
        assertFalse(AgentConversation.offered(codexServing, null, claude(preset = "shell")))
        assertFalse(AgentConversation.offered(serving, null, claude(paneMode = "agent")))
        assertFalse(AgentConversation.offered(serving, null, claude(state = "exited")))
        assertFalse(AgentConversation.offered(serving, null, null))
        // A runner without the capabilities.
        assertFalse(AgentConversation.offered(build(Capability.TERMINALS), null, claude()))
    }

    @Test
    fun `a reconnect that has not read the build yet keeps the view, gated on the last known build`() {
        // `daemon` is null from the moment a link comes up until `host` answers.
        assertTrue(AgentConversation.offered(null, serving, claude()))
        // A last build that never served it doesn't conjure one.
        assertFalse(AgentConversation.offered(null, build(Capability.TERMINALS), claude()))
        // The fresh build wins over the last: a runner that turned it off stops offering.
        assertFalse(AgentConversation.offered(build(Capability.TERMINALS), serving, claude()))
    }

    @Test
    fun `a codex pane's words name codex`() {
        assertEquals("Codex", AgentConversation.agentName("codex"))
        assertEquals("Claude", AgentConversation.agentName("claude"))
        assertFalse(AgentConversation.pressesKeys("codex"))
        assertTrue(AgentConversation.pressesKeys("claude"))
        for (what in listOf("busy", "left_at_shell", "unconfirmed", "not_running", "unconfirmable", "picker", "too_tall", "command")) {
            val said = (AgentConversation.issue(SendFailure.Refused(what), agent = "Codex") as SendIssue.Said).words
            assertTrue("$what: $said", said.contains("Codex"))
            assertFalse(what, said.contains("Claude"))
        }
        assertEquals("Codex is showing something only the terminal can.", AgentConversation.handoff("Codex"))
        assertTrue(AgentConversation.picker("Codex").contains("@ or $"))
    }

    @Test
    fun `the settings row shows to a host admin on a runner that says where the setting stands`() {
        val admin = build(Capability.PROJECTOR_SETTING, scope = "host_admin")
        assertTrue(AgentConversation.offersSetting(admin, false))
        assertFalse(AgentConversation.offersSetting(admin, null))
        assertFalse(AgentConversation.offersSetting(build(Capability.PROJECTOR_SETTING, scope = "control"), true))
        assertFalse(AgentConversation.offersSetting(build(Capability.TERMINALS, scope = "host_admin"), true))
        assertFalse(AgentConversation.offersSetting(null, true))
    }

    @Test
    fun `a draft is one line, and a command is refused`() {
        assertEquals("a b c d", AgentConversation.flattened("a\nb\r\nc\rd"))
        assertEquals("plain", AgentConversation.flattened("plain"))
        for (symbol in "/!#@&$?\\") assertTrue(AgentConversation.isCommand("${symbol}x"))
        assertFalse(AgentConversation.isCommand("fix the build"))
        assertFalse(AgentConversation.isCommand(""))
    }

    @Test
    fun `refusals map as the Mac maps them`() {
        fun said(what: String?, word: String? = null) = AgentConversation.issue(SendFailure.Refused(what, word))
        assertEquals(SendIssue.Handoff, said("dialog"))
        assertEquals(SendIssue.Handoff, said("prompt"))
        assertEquals(SendIssue.DraftInTerminal, said("draft"))
        assertEquals(SendIssue.Said(AgentConversation.TOO_LONG), said("too_long"))
        assertEquals(SendIssue.Said(AgentConversation.COMMAND_REFUSED), said("command"))
        assertEquals(SendIssue.Said("Claude isn’t running in this pane."), said("not_running"))
        assertEquals(
            SendIssue.Said("Someone typed in the terminal in the last 3 seconds, so the message wasn’t sent. Try again once they stop."),
            said("typing"),
        )
        assertEquals(SendIssue.Said("The message wasn’t sent."), said("something-new"))
        assertEquals(SendIssue.Said("The message wasn’t sent."), said(null))
        // A read-only device is told so, not "wasn't sent" on every try.
        assertEquals(SendIssue.Said("This device can’t send messages to this runner."), said(null, "scope-denied"))
    }

    @Test
    fun `a send that may have arrived never says it wasn't sent`() {
        val may = SendIssue.Said(AgentConversation.MAY_HAVE_BEEN_SENT)
        assertEquals(may, AgentConversation.issue(SendFailure.TimedOut))
        assertEquals(may, AgentConversation.issue(SendFailure.Lost(notSent = false)))
        assertEquals(
            SendIssue.Said("The runner isn’t connected, so the message wasn’t sent."),
            AgentConversation.issue(SendFailure.Lost(notSent = true)),
        )
        assertTrue(AgentConversation.MAY_HAVE_BEEN_SENT.contains("may have been sent"))
    }

    @Test
    fun `errors out of the core map to failures, and what no runner said reads as maybe sent`() {
        // The core's own deadline: no `code`, `timed_out`, read as the timed-out word.
        assertEquals(SendFailure.TimedOut, AgentConversation.failure(CoreException("late", RunnerRefusal.TIMED_OUT_WORD)))
        assertEquals(SendFailure.Lost(true), AgentConversation.failure(DisconnectedException("gone", notSent = true)))
        assertEquals(SendFailure.Lost(false), AgentConversation.failure(DisconnectedException("gone", notSent = false)))
        assertEquals(SendFailure.Refused("dialog", "invalid-argument"), AgentConversation.failure(CoreException("no", "invalid-argument", "dialog")))
        assertEquals(SendFailure.Refused(null, "scope-denied"), AgentConversation.failure(CoreException("no", "scope-denied")))
        // The core closed under the call, or an answer that couldn't be read:
        // a runner may well have typed it (ov-373 review 1, item 4).
        assertEquals(SendFailure.TimedOut, AgentConversation.failure(CoreException("The connection was closed.")))
        assertEquals(SendFailure.TimedOut, AgentConversation.failure(IllegalStateException("what")))
    }

    @Test
    fun `a Queued echo settles once the transcript shows it, as a Queued row or as the turn it became`() {
        fun turn(prompt: String) =
            AgentRow("t", 1, 1, kind = AgentRow.Kind.OfTurn(AgentRow.Turn(prompt, "Queued")))
        fun queued(text: String) =
            AgentRow("q", 2, 2, kind = AgentRow.Kind.OfQueued(AgentRow.Queued(text, "Waiting")))
        assertEquals(listOf("a", "b"), AgentConversation.unsettled(listOf("a", "b"), emptyList()))
        assertEquals(listOf("b"), AgentConversation.unsettled(listOf("a", "b"), listOf(queued("a"))))
        assertEquals(emptyList<String>(), AgentConversation.unsettled(listOf("a", "b"), listOf(turn("a"), queued("b"))))
    }

    @Test
    fun `a turn nobody typed is a notice`() {
        fun turn(origin: String) = AgentRow.Turn("\"build\" finished", origin)
        assertTrue(AgentConversation.isNotice(turn("Notification")))
        assertTrue(AgentConversation.isNotice(turn("System")))
        assertFalse(AgentConversation.isNotice(turn("Typed")))
        assertFalse(AgentConversation.isNotice(turn("Queued")))
        assertEquals("build finished", AgentConversation.noticeText(turn("Notification")))
    }

    @Test
    fun `the words for rows`() {
        assertEquals("0:04", AgentConversation.short(4_000))
        assertEquals("1:12", AgentConversation.short(72_000))
        assertEquals("2:03:09", AgentConversation.short(7_389_000))
        assertEquals("General purpose", AgentConversation.agentType("general-purpose"))
        assertEquals("Agent", AgentConversation.agentType(""))
        fun outcome(o: AgentRow.Turn.Outcome?, ms: Long? = null) =
            AgentConversation.outcome(AgentRow.Turn("p", "Typed", durationMs = ms, outcome = o))
        assertEquals("Took 1:00", outcome(AgentRow.Turn.Outcome.Finished, 60_000))
        assertEquals("Done", outcome(AgentRow.Turn.Outcome.Finished))
        assertEquals("Failed: API error", outcome(AgentRow.Turn.Outcome.Failed("API error")))
        assertEquals("Failed", outcome(AgentRow.Turn.Outcome.Failed("")))
        assertEquals(null, outcome(null))
        assertEquals("Some of this session couldn’t be read.", AgentConversation.gap(AgentRow.Gap("Unparsed", 2)))
        assertEquals("A line of this session couldn’t be read.", AgentConversation.gap(AgentRow.Gap("Unparsed", 1)))
        assertEquals("Sent from the queue", AgentConversation.queuedLabel("Sent"))
        assertEquals("Queued", AgentConversation.queuedLabel("Waiting"))
    }

    /** ov-404: the Mac's words for an image refused, and which limit it hit. */
    @Test
    fun `a refused image says which limit it hit`() {
        fun said(what: String, command: Boolean = false) = AgentConversation.issue(SendFailure.Refused(what), command)
        assertEquals(SendIssue.Said(AgentConversation.IMAGE_TOO_LARGE), said("image_too_large"))
        assertEquals(SendIssue.Said(AgentConversation.IMAGES_TOO_LARGE), said("images_too_large"))
        assertEquals(SendIssue.Said(AgentConversation.TOO_MANY_IMAGES), said("images"))
        assertEquals("a slash command can't carry any, however few", SendIssue.Said(AgentConversation.COMMAND_WITH_IMAGES), said("images", command = true))
        assertTrue(AgentConversation.IMAGE_TOO_LARGE != AgentConversation.IMAGES_TOO_LARGE)
        assertEquals(SendIssue.Said(AgentConversation.BACKSLASH), said("backslash"))
        assertEquals(SendIssue.Panel, said("handoff"))
        assertEquals(SendIssue.Said(AgentConversation.UNCONFIRMED), said("unconfirmed"))
    }

    @Test
    fun `the limits follow whether the runner has compose`() {
        assertEquals(500, AgentConversation.longest(rich = false))
        assertEquals(100_000, AgentConversation.longest(rich = true))
        assertTrue(AgentConversation.tooLong(rich = true).contains("100,000"))
        assertTrue(AgentConversation.tooLong(rich = false).contains("500"))
    }

    @Test
    fun `an echo of images settles against the row claude wrote`() {
        assertEquals("[Image] [Image] look", AgentConversation.echo("look", 2))
        assertEquals("[Image]", AgentConversation.echo("  ", 1))
        fun row(text: String) = AgentRow("q", 1, 1, kind = AgentRow.Kind.OfQueued(AgentRow.Queued(text, "Waiting", null)))
        assertEquals(emptyList<String>(), AgentConversation.unsettled(listOf("[Image] [Image] look"), listOf(row("[Image #1] [Image #2] look"))))
        assertEquals(1, AgentConversation.unsettled(listOf("[Image] [Image] look"), listOf(row("[Image #1] look"))).size)
        assertEquals(1, AgentConversation.unsettled(listOf("look"), listOf(row("look elsewhere"))).size)
    }

    private fun turn(outcome: AgentRow.Turn.Outcome? = null, activity: String?) =
        AgentRow.Turn("p", "Typed", outcome = outcome, activity = activity)

    /** Stop is for a turn claude is working on; under a dialog its Esc would answer No. */
    @Test
    fun `Stop is offered only while claude works`() {
        assertTrue(AgentConversation.isWorking(turn(activity = "Busy")))
        assertFalse("hidden under a dialog", AgentConversation.isWorking(turn(activity = "Waiting")))
        assertFalse(AgentConversation.isWorking(turn(activity = "Idle")))
        assertFalse(AgentConversation.isWorking(turn(activity = "Shell")))
        assertFalse("a turn that ended", AgentConversation.isWorking(turn(AgentRow.Turn.Outcome.Finished, "Busy")))
        assertFalse(AgentConversation.isWorking(null))
    }

    @Test
    fun `a key that did nothing needed says nothing, and the others say what to do`() {
        fun key(what: String, key: AgentConversation.PaneKey) = AgentConversation.keyIssue(SendFailure.Refused(what), key)
        val stop = AgentConversation.PaneKey.Stop
        val now = AgentConversation.PaneKey.SendNow
        assertNull(key("idle", stop))
        assertNull(key("too_soon", now))
        assertEquals(SendIssue.Handoff, key("prompt", stop))
        assertEquals(SendIssue.Said("Nothing is waiting in Claude’s queue."), key("nothing_queued", now))
        assertEquals(SendIssue.Said("Claude is starting a step. Try Stop again in a moment."), key("settling", stop))
        assertEquals(SendIssue.Said("Claude is starting a step. Try Send now again in a moment."), key("settling", now))
        assertEquals(
            SendIssue.Said("Claude didn’t confirm it stopped. It may have stopped; check the terminal before pressing again."),
            key("unconfirmed", stop),
        )
        assertEquals(SendIssue.Said("The runner didn’t answer in time. Check the terminal."), AgentConversation.keyIssue(SendFailure.TimedOut, stop))
        assertEquals(SendIssue.Said("The runner isn’t connected. Use the terminal."), AgentConversation.keyIssue(SendFailure.Lost(true), stop))
        assertEquals(SendIssue.Said("Far Cooler can’t stop Claude safely from here. Use the terminal."), key("mystery", stop))
    }
}
