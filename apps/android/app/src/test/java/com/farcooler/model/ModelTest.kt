package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
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
    ) = Terminal(id = id, title = title, preset = preset, state = state, activity = activity)

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
        // Which is what sends the app to its worktree list instead.
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
}
