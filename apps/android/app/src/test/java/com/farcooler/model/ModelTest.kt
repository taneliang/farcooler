package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The derivations the fleet list, the tab strip and the terminal title all read
 * from. Ported from the reasoning in `apps/ios/FarCooler/Model.swift`, which
 * itself ports the Mac's — three surfaces reading one answer is the whole point,
 * so the answer is tested rather than eyeballed on each of them.
 */
class ModelTest {
    private fun terminal(
        id: String = "t",
        title: String = "",
        preset: String = "",
        state: String = "running",
        activity: String? = null,
        turnFailed: Boolean? = null,
        exitCode: Int? = null,
        exitSignal: Int? = null,
        activitySince: Double? = null,
        turnStartedAt: Double? = null,
        feed: List<String>? = null,
        said: String? = null,
        subagents: List<String>? = null,
        line: String? = null,
        rank: Long? = null,
        paneMode: String? = null,
    ) = Terminal(
        id = id,
        title = title,
        preset = preset,
        state = state,
        activity = activity,
        turnFailed = turnFailed,
        exitCode = exitCode,
        exitSignal = exitSignal,
        activitySince = activitySince,
        turnStartedAt = turnStartedAt,
        feed = feed,
        said = said,
        subagents = subagents,
        line = line,
        rank = rank,
        paneMode = paneMode,
    )

    /** A round moment to measure ages against, so the arithmetic reads. */
    private val now = 1_756_000_000_000L

    private fun secondsAgo(seconds: Long): Double = (now - seconds * 1000).toDouble()

    @Test
    fun aTerminalIsNamedAfterWhatIsRunningInIt() {
        assertEquals("claude", terminal(preset = "claude").label)
        // The host reports whatever tmux sees running, so the same plain shell
        // arrives as `zsh` from a pane the watcher has looked at and as `shell`
        // from one it has not.
        assertEquals("shell", terminal(preset = "zsh").label)
        assertEquals("shell", terminal(preset = "-zsh").label)
        assertEquals("shell", terminal(preset = "").label)
    }

    @Test
    fun aNamedConversationBeatsThePresetAndNeedsNoOrdinal() {
        // A fleet of agents all reading "claude 1", "claude 2" names the one
        // thing every pane has in common and therefore says nothing about which
        // is which.
        val named = terminal(title = "Fix the parser", preset = "claude")
        assertEquals("Fix the parser", named.label)
        assertEquals("Fix the parser", named.displayName(2))
    }

    @Test
    fun theAutomaticTitleIsNotATitle() {
        assertEquals("claude", terminal(title = "Terminal 12", preset = "claude").label)
        assertEquals("claude", terminal(title = "Terminal", preset = "claude").label)
    }

    @Test
    fun identicalSiblingsAreNumberedAndUniqueOnesAreNot() {
        // Two `claude` panes in one workspace are genuinely alike; a lone
        // `shell` numbered "1" answers a question nobody asked.
        val workspace = Workspace(
            id = "w",
            terminals = listOf(
                terminal(id = "a", preset = "claude"),
                terminal(id = "b", preset = "claude"),
                terminal(id = "c", preset = "zsh"),
            ),
        )
        val ordinals = workspace.ordinals()
        assertEquals(1, ordinals["a"])
        assertEquals(2, ordinals["b"])
        assertNull(ordinals["c"])
    }

    @Test
    fun onlyBlockedAndDoneAreWorthInterrupting() {
        // A product that buzzes for the normal case is one people turn off,
        // after which it cannot tell them the thing that mattered.
        assertEquals(true, AgentActivity.BLOCKED.wantsAttention)
        assertEquals(true, AgentActivity.DONE.wantsAttention)
        assertEquals(false, AgentActivity.WORKING.wantsAttention)
        assertEquals(false, AgentActivity.IDLE.wantsAttention)
        assertEquals(false, AgentActivity.NONE.wantsAttention)
    }

    @Test
    fun anActivityThisBuildHasNeverHeardOfIsUnknownRatherThanNothing() {
        // NONE means "not an agent at all", which is a different claim from
        // "this build cannot name what it is doing".
        assertEquals(AgentActivity.UNKNOWN, AgentActivity.parse("teleporting"))
        assertEquals(AgentActivity.NONE, AgentActivity.parse(null))
    }

    @Test
    fun theLandingTerminalPrefersOneThatNeedsYou() {
        // An agent waiting on you outranks everything else, because that is the
        // whole reason to have opened the app.
        val fleet = Fleet(
            workspaces = listOf(
                Workspace(
                    id = "w",
                    terminals = listOf(
                        terminal(id = "running", state = "running"),
                        terminal(id = "blocked", state = "running", activity = "blocked"),
                    ),
                )
            )
        )
        assertEquals("blocked", fleet.landingTerminal?.id)
    }

    @Test
    fun aRunningTerminalBeatsOneThatExited() {
        val fleet = Fleet(
            workspaces = listOf(
                Workspace(
                    id = "w",
                    terminals = listOf(
                        terminal(id = "dead", state = "exited"),
                        terminal(id = "alive", state = "running"),
                    ),
                )
            )
        )
        assertEquals("alive", fleet.landingTerminal?.id)
    }

    @Test
    fun aHostWithNoTerminalsHasNothingToLandOn() {
        // Which is what sends the app to its workspace list instead.
        assertNull(Fleet(workspaces = listOf(Workspace(id = "w"))).landingTerminal)
    }

    @Test
    fun aChatIsOnlyOfferedWhenTheHostSaysSo() {
        // Absent means "do not offer": a switch that came back as a different
        // agent is worse than no switch at all.
        assertEquals(false, terminal().canSwitchPaneMode)
        assertEquals(false, Terminal(id = "t", chatCapable = false).canSwitchPaneMode)
        assertEquals(true, Terminal(id = "t", chatCapable = true).canSwitchPaneMode)
    }

    // MARK: - The four bands
    //
    // Everything below is a pure function of a decoded pane and, where a clock
    // is involved, a moment. That is the whole reason they are testable at all
    // and the reason they live on `Terminal` rather than inside the row: a
    // layout wants an emulator, and these want an assertion.

    @Test
    fun theSignalLineIsWhatTheHostComposed() {
        // Not re-derived here from `blockedQuestion`, `planDone` or `activity`.
        // The priority between those three rungs is decided in
        // `farcooler_core::feed::line`, because a Mac, a phone and a watch
        // deciding it separately is three surfaces disagreeing about one pane.
        assertEquals("3/7 · Designing test matrix", terminal(line = "3/7 · Designing test matrix").signalLine)
        // Whitespace is not a line. A blank one draws an empty row and makes
        // every surface taller for nothing.
        assertEquals("", terminal(line = "   \n ").signalLine)
        assertEquals("", terminal(line = null).signalLine)
    }

    @Test
    fun theTranscriptKeepsAtMostThreeAndKeepsTheLatest() {
        // The daemon already keeps three; the cap is repeated so a host that
        // ever sent four could not make one row twice the height of every other
        // row in the list.
        val four = terminal(feed = listOf("one", "two", "three", "four"))
        assertEquals(listOf("two", "three", "four"), four.recentSteps)
        // Blank entries are dropped rather than drawn, and the cap is applied
        // to what is left rather than to what arrived.
        val padded = terminal(feed = listOf("one", "  ", "two", "", "three"))
        assertEquals(listOf("one", "two", "three"), padded.recentSteps)
    }

    @Test
    fun theTranscriptSurvivesTheAgentGoingIdle() {
        // "What did this do while I was away" is exactly when the summary is
        // worth most, and a row that shed its lines on going idle would also
        // mean the list rearranging itself under somebody reading it.
        val idle = terminal(activity = "idle", feed = listOf("Rewrote the poller."))
        assertEquals(listOf("Rewrote the poller."), idle.recentSteps)
    }

    @Test
    fun aNotificationQuotesTheHeadOfTheSentenceAndNotTheTailOfTheWindow() {
        // `said` and the feed's last line are cut from ONE message at opposite
        // ends. A feed entry is a wrapped ROW, so its last line is the last
        // forty characters of the window — which is how a lock screen came to
        // read "batches to avoid N+1 shits."
        val both = terminal(
            feed = listOf("batches to avoid N+1 shits."),
            said = "More shit, shipped in carefully authorized batches.",
        )
        assertEquals("More shit, shipped in carefully authorized batches.", both.lastSaid)
        // The tail is the fallback and only that: a runner on an older daemon
        // sends no `said`, and a worse sentence beats nothing.
        assertEquals("batches to avoid N+1 shits.", terminal(feed = listOf("batches to avoid N+1 shits.")).lastSaid)
        assertNull(terminal().lastSaid)
        // Whitespace is not something said.
        assertNull(terminal(said = "  ").lastSaid)
    }

    @Test
    fun atMostThreeSubagentsAreNamed() {
        // Their COUNT is already inside the signal line; these are the names,
        // and three is what fits beside a row on a phone.
        val many = terminal(subagents = listOf("explore", "plan", "review", "ship"))
        assertEquals(listOf("explore", "plan", "review"), many.runningSubagents)
        assertEquals(listOf("explore"), terminal(subagents = listOf("explore", " ")).runningSubagents)
        assertEquals(emptyList<String>(), terminal().runningSubagents)
    }

    @Test
    fun aFailedTurnAndAFailedProcessAreDifferentQuestions() {
        // One is about the agent's last turn, read from its session log; the
        // other is about the command, read from how its process exited. A
        // `cargo build` that returned 101 has no turns at all.
        val failedTurn = terminal(activity = "done", turnFailed = true)
        assertTrue(failedTurn.turnDidFail)
        assertFalse(failedTurn.runDidFail)
        assertEquals("Failed", failedTurn.activityLabel)

        val failedRun = terminal(state = "exited", exitCode = 101)
        assertTrue(failedRun.runDidFail)
        assertFalse(failedRun.turnDidFail)
    }

    @Test
    fun aTurnStillRunningIsNotFailingAndOneAlreadySeenHasBeenTold() {
        // The failure belongs to the turn that ENDED. Only `done` can answer it.
        assertFalse(terminal(activity = "working", turnFailed = true).turnDidFail)
        assertFalse(terminal(activity = "idle", turnFailed = true).turnDidFail)
    }

    @Test
    fun anAbsentExitStatusIsNotAFailure() {
        // The clause that matters: an older daemon sends no exit status at all,
        // and reading nothing as broken would mark every finished terminal on
        // the runner as failed.
        assertFalse(terminal(state = "exited").runDidFail)
        assertFalse(terminal(state = "exited", exitCode = 0).runDidFail)
        assertTrue(terminal(state = "exited", exitSignal = 9).runDidFail)
        // And a process that has not ended cannot be asked how it ended.
        assertFalse(terminal(state = "running", exitCode = 101).runDidFail)
    }

    @Test
    fun theTwoClocksAnswerDifferentQuestions() {
        // `Working` is only ever mid-turn, so the TURN clock is the honest
        // answer to "how long has this been going". `Blocked` wants the STATE
        // clock, because a prompt held for twenty minutes is the thing to
        // notice, not how long the turn around it has run. Conflating them is
        // the bug the two exist to fix.
        val working = terminal(
            activity = "working",
            activitySince = secondsAgo(30),
            turnStartedAt = secondsAgo(720),
        )
        assertEquals("12m", working.displayDuration(now))

        val blocked = terminal(
            activity = "blocked",
            activitySince = secondsAgo(120),
            turnStartedAt = secondsAgo(3600),
        )
        assertEquals("2m", blocked.displayDuration(now))
    }

    @Test
    fun idleAndDoneHaveNoClockAtAll() {
        // "Idle for three days" is noise. It is also what decides whether a row
        // starts a ticking coroutine, which is why `hasClock` is asked before
        // the duration rather than derived from it.
        val idle = terminal(activity = "idle", activitySince = secondsAgo(300))
        assertFalse(idle.hasClock)
        assertNull(idle.displayDuration(now))
        assertFalse(terminal(activity = "done", activitySince = secondsAgo(300)).hasClock)
        assertTrue(terminal(activity = "working").hasClock)
        assertTrue(terminal(activity = "blocked").hasClock)
    }

    @Test
    fun aRowDoesNotFlickerOneSecondOnItsWayToSayingSomethingUseful() {
        val fresh = terminal(activity = "blocked", activitySince = secondsAgo(2))
        assertNull(fresh.displayDuration(now))
        // And a runner whose wall clock is ahead of this phone's falls in the
        // same branch rather than printing a negative age.
        val ahead = terminal(activity = "blocked", activitySince = secondsAgo(-90))
        assertNull(ahead.displayDuration(now))
    }

    @Test
    fun elapsedTimeIsTheSameThreeUnitsEverySurfacePrints() {
        fun blockedFor(seconds: Long) =
            terminal(activity = "blocked", activitySince = secondsAgo(seconds)).displayDuration(now)
        assertEquals("5s", blockedFor(5))
        assertEquals("59s", blockedFor(59))
        assertEquals("1m", blockedFor(60))
        assertEquals("59m", blockedFor(3599))
        assertEquals("1h", blockedFor(3600))
        assertEquals("26h", blockedFor(94_000))
    }

    @Test
    fun theRowSaysTheStateAndItsAgeAndNeverRestatesTheDot() {
        // The string this whole band replaced was `state.lowercase()` — the raw
        // wire word, restating the process dot immediately to its left.
        val working = terminal(activity = "working", turnStartedAt = secondsAgo(720))
        assertEquals("Working 12m", working.rowStatus(now))
        // The label alone until there is an age worth printing.
        assertEquals("Needs you", terminal(activity = "blocked").rowStatus(now))
        assertEquals("Failed", terminal(activity = "done", turnFailed = true).rowStatus(now))
    }

    @Test
    fun aLiveShellSaysNothingAndADeadOneSaysWhatHappenedToIt() {
        // Running is the ordinary case and the dot draws nothing for it either,
        // so a live shell is a row with a name on it — which is the point of
        // the silence, not a fact withheld.
        assertNull(terminal(preset = "zsh").rowStatus(now))
        assertEquals("exited", terminal(preset = "zsh", state = "exited").rowStatus(now))
        assertEquals("lost", terminal(preset = "zsh", state = "lost").rowStatus(now))
        // An agent this build cannot name is not an agent it can describe.
        assertNull(terminal(preset = "claude", activity = "teleporting").rowStatus(now))
    }

    @Test
    fun anUnrankedPaneSortsLastRatherThanFirst() {
        // A daemon too old to send a rank is a daemon that cannot tell us this
        // pane is urgent, and guessing that it is would put an unknown above a
        // known blocked agent.
        assertEquals(42L, terminal(rank = 42).sortRank)
        assertEquals(Long.MAX_VALUE, terminal().sortRank)
    }

    @Test
    fun aChangesPaneIsNeitherAChatNorATerminal() {
        // Which is the defect it exists to let a later phase fix: a `changes`
        // pane falls past `isAgentPane` to the VT renderer and is drawn as a
        // grid of whatever bytes are on a pane that is not a tty.
        val changes = terminal(paneMode = "changes")
        assertTrue(changes.isChangesPane)
        assertFalse(changes.isAgentPane)
        assertFalse(terminal(paneMode = "agent").isChangesPane)
        assertFalse(terminal().isChangesPane)
    }
}
