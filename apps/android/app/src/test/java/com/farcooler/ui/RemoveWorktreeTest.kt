package com.farcooler.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The two rules standing in front of the most destructive thing this app can do.
 *
 * Neither of them is what makes a worktree safe — `Service::remove_worktree`
 * refuses the primary checkout twice and an unreachable tmux once, before it
 * touches a terminal or a directory, and those refusals are covered by tests in
 * `crates/daemon`. These are what keep somebody out of a destructive ceremony
 * that could never have succeeded, which is the part `07e75e8` is a fix for: on
 * iOS the flag that hides the menu item decoded wrong, so the phone offered to
 * remove the repository's own checkout and walked people through typing its name
 * before the runner said no.
 */
class RemoveWorktreeTest {

    /**
     * The typed name has to be the worktree's, and an empty expectation is never
     * a match.
     *
     * `crates/daemon/src/rpc.rs` compares `typed_confirmation.trim()` against
     * `ws.name()`, which is the same string `wire::workspace` sends as the
     * fleet's `task_name` — so this comparison and the runner's are about the
     * same name, which is the fact worth pinning.
     */
    @Test
    fun `a removal name must match the worktree's own, trimmed`() {
        assertTrue(removalNameMatches("rate limiting", "rate limiting"))
        // A phone keyboard adds a trailing space; the runner trims too, so
        // refusing over one would be refusing over the keyboard.
        assertTrue(removalNameMatches("rate limiting", "  rate limiting "))
        assertFalse(removalNameMatches("rate limiting", "Rate Limiting"))
        assertFalse(removalNameMatches("rate limiting", "rate"))
        assertFalse(removalNameMatches("rate limiting", ""))
        // The one input this gate must never accept: a workspace whose name did
        // not arrive would otherwise be removable by typing nothing at all.
        assertFalse(removalNameMatches("", ""))
        assertFalse(removalNameMatches("", "   "))
    }

    /**
     * The dialog says what will happen, or why it can't — and the second sentence
     * is the whole explanation for a switched-off button.
     */
    @Test
    fun `the prompt names the runner and never leaks a wire word`() {
        val ok = removalPrompt("studio", tmuxDown = false)
        assertTrue(ok.contains("studio"))
        // The reassurance is the half that decides whether this is frightening,
        // and it is true: the daemon removes the worktree and never the branch.
        assertTrue(ok.contains("branch"))
        assertFalse(ok.contains("tmux"))

        val blocked = removalPrompt("studio", tmuxDown = true)
        assertTrue(blocked.contains("studio"))
        assertTrue(blocked.contains("tmux"))
        // Not the fleet footer's three words. This sentence has to explain a
        // control, so it says what Far Cooler cannot do and why.
        assertFalse(blocked.contains("tmux unavailable"))

        // Two different sentences, not one with a clause bolted on. A prompt
        // that offered removal and then explained it could not would be the
        // ceremony this whole gate exists to prevent, written smaller.
        assertFalse(ok == blocked)
        assertEquals(2, setOf(ok, blocked).size)
    }
}
