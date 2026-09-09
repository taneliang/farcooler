package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What this phone says before it closes a terminal.
 *
 * Closing is the only irreversible thing either phone can do to a pane: it is
 * killed and its record is deleted, `remain-on-exit` takes the dead rectangle
 * with it, and there is no bin. The sentence in front of that is the last thing
 * anybody gets to read, and a duration that says `0s`, an agent named `shell`
 * because the preset was empty, or a claim that something is running when it
 * exited an hour ago are all defects that look perfectly normal on a screenshot.
 *
 * **The same table as iOS's `ShellCloseTests`, in the same order and against the
 * same words.** The two phones differ in gesture and must not differ in what
 * they say, so a change made on one side and not the other fails on whichever
 * side it was left out of.
 */
class ShellCloseTest {
    /** A fixed instant, so every duration below is a number rather than a race. */
    private val now = 1_757_000_000_000L

    private fun ago(seconds: Double) = now - seconds * 1000

    private fun terminal(
        state: String,
        preset: String = "claude",
        title: String = "",
        activity: String? = null,
        activitySince: Double? = null,
        turnStartedAt: Double? = null,
    ) = Terminal(
        id = "t1",
        short = "t1",
        title = title,
        preset = preset,
        state = state,
        activity = activity,
        activitySince = activitySince,
        turnStartedAt = turnStartedAt,
    )

    /**
     * **A stopped pane is closed without a word.**
     *
     * Null is not "no opinion" — it is the answer that means close it now, and
     * the tab strip's menu reads it that way. Every state the daemon will REMOVE
     * without complaint is here, because the confirmation exists to cover the
     * stop that has to happen first, and a pane with nothing to stop is a pane
     * with nothing to ask about.
     */
    @Test
    fun `a pane with nothing running is not worth asking about`() {
        for (state in listOf("exited", "error", "lost", "something-new")) {
            assertFalse(state, ShellClose.mustAsk(terminal(state)))
            assertNull(state, ShellClose.question(terminal(state), now))
        }
    }

    /**
     * **The two states the daemon refuses to remove are the two that ask.**
     *
     * `Service::remove_terminal` answers `RunningProcesses` for `Running` and
     * `Starting`, which is what makes the stop mandatory — so those are exactly
     * the panes where closing interrupts something. The pair is one fact, and
     * this is what keeps the phone's copy of it in step with the daemon's.
     */
    @Test
    fun `a live pane always asks`() {
        for (state in listOf("running", "starting")) {
            assertTrue(state, ShellClose.mustAsk(terminal(state)))
        }
    }

    /**
     * **The dialog names the pane in its title and the agent in its body.**
     *
     * Two different names on purpose. The title is [Terminal.label] — the
     * conversation's own name where the agent has given it one — because that is
     * the string on the chip that was held down, and a confirmation that renamed
     * the thing it is about would be asking about something else. The body is
     * the COMMAND, which is what is actually going to be killed.
     */
    @Test
    fun `a running agent is named twice and timed once`() {
        val question = ShellClose.question(
            terminal(
                state = "running",
                preset = "claude",
                title = "Rewrite the parser",
                activity = "working",
                activitySince = ago(20.0),
                turnStartedAt = ago(754.0),
            ),
            now,
        )!!
        assertEquals("Close “Rewrite the parser”?", question.title)
        assertEquals(
            "It’s running claude, which has been working for 12m. " +
                "Closing stops it and removes the tab. There’s no undo.",
            question.message,
        )
    }

    /**
     * **A working agent is timed by its TURN and a blocked one by its STATE.**
     *
     * The two clocks answer different questions and conflating them is the bug
     * [Terminal.displayDuration] exists to fix: `working` is only ever mid-turn,
     * so the turn clock is the honest answer to "how long has this been going",
     * while a prompt held for twenty minutes is the thing to notice about a
     * blocked one rather than how long the turn around it has run. Both fixtures
     * carry BOTH timestamps with different values — a version that read one
     * clock for both would answer 12m twice.
     */
    @Test
    fun `a blocked agent is timed by how long it has been waiting`() {
        val question = ShellClose.question(
            terminal(
                state = "running",
                preset = "codex",
                activity = "blocked",
                activitySince = ago(1300.0),
                turnStartedAt = ago(9000.0),
            ),
            now,
        )!!
        assertEquals("Close “codex”?", question.title)
        assertEquals(
            "It’s running codex, which has been waiting on you for 21m. " +
                "Closing stops it and removes the tab. There’s no undo.",
            question.message,
        )
    }

    /**
     * **An agent with no clock loses the clause, not the sentence.**
     *
     * Three ways to get here and all of them are ordinary: a plain shell, which
     * has no agent and therefore no activity at all; an idle or finished agent,
     * whose age is noise — "idle for three days" is not a reason to keep a pane;
     * and a runner too old to send a timestamp, where null means "nobody said"
     * and must never be rendered as "just now".
     *
     * What it must not do is say `0s`. A duration under five seconds is null
     * from [Terminal.brief] for that reason, and this asserts the sentence
     * survives it rather than growing a hole.
     */
    @Test
    fun `a pane with no clock still gets a sentence`() {
        val plain = "Closing stops it and removes the tab. There’s no undo."

        assertEquals(
            "It’s running shell. $plain",
            ShellClose.question(terminal(state = "running", preset = "zsh"), now)!!.message,
        )
        assertEquals(
            "It’s running claude. $plain",
            ShellClose.question(
                terminal(state = "running", activity = "idle", activitySince = ago(90000.0)),
                now,
            )!!.message,
        )
        assertEquals(
            "It’s running claude. $plain",
            ShellClose.question(
                terminal(state = "running", activity = "working", turnStartedAt = ago(2.0)),
                now,
            )!!.message,
        )
    }

    /**
     * **The button says the noun, in title case, and never "Delete".**
     *
     * The word is Close rather than Delete because that is what the product
     * calls it everywhere else: the Mac's menu item is `Close Terminal` and
     * iOS's swipe confirms with the same two words. A phone that called the same
     * act deleting would be a third vocabulary for one thing.
     */
    @Test
    fun `the confirm button names what it closes`() {
        assertEquals("Close Terminal", ShellClose.CONFIRM)
    }
}
