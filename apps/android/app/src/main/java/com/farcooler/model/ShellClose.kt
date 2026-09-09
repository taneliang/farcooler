package com.farcooler.model

/**
 * Closing a terminal: what it costs, and what a phone has to say first.
 *
 * **The rule and the words, in one place, because both phones ship them.**
 * Closing is two calls — `terminal.stop` then `terminal.remove` — and the
 * daemon refuses the second one on a live pane (`Service::remove_terminal`
 * answers `RunningProcesses` for `Running` and `Starting`). So the stop is not
 * a courtesy, it is what makes the removal legal, and a phone that only stopped
 * would leave the dead rectangle `remain-on-exit` deliberately keeps.
 *
 * **A stop is not undoable and nothing here pretends otherwise.** The pane is
 * killed and the record is deleted; there is no bin to fish it out of. That is
 * what earns the confirmation, and it is also why the confirmation is only for
 * the case that has something to lose: a pane whose process has already gone
 * has no agent to interrupt, so asking about it would be a tax charged on the
 * harmless case to protect the rare one.
 *
 * **A port of `AgentKit/ShellClose.swift`, sentence for sentence**, and the
 * duplication is the point. The two phones differ in GESTURE — a swipe on a
 * column row there, a menu on a tab chip here, each platform's own convention —
 * and they must not differ in what they tell somebody they are about to lose.
 * `ShellCloseTest` asserts the same table as `ShellCloseTests` so the two can be
 * read side by side and a change to one that is not made to the other fails on
 * whichever side it was left out of.
 */
object ShellClose {
    /** The sheet a running pane earns, or null for one that has already exited. */
    data class Question(
        /**
         * Names the pane, because the menu that got here named nothing: a chip
         * was held down and a red word appeared under a thumb.
         */
        val title: String,
        /**
         * Names the agent and how long it has been going, then says what
         * closing does and that it cannot be taken back.
         */
        val message: String,
    )

    /**
     * The button that does it. Title case, and it says the noun: a bare `Close`
     * in a dialog raised by a long press is a word with nothing attached to it.
     */
    const val CONFIRM = "Close Terminal"

    /**
     * Whether closing this pane must be confirmed first.
     *
     * **The same two states `remove_terminal` refuses**, and that is the whole
     * definition rather than a coincidence worth restating: the confirmation
     * exists because the close has to STOP something, and the only panes it has
     * to stop are the ones the daemon will not remove while they live. `Lost`,
     * `Exited`, `Error` and `Unknown` have no process to interrupt.
     */
    fun mustAsk(terminal: Terminal): Boolean = when (StateKind.parse(terminal.state)) {
        StateKind.RUNNING, StateKind.STARTING -> true
        StateKind.EXITED, StateKind.ERROR, StateKind.LOST, StateKind.UNKNOWN -> false
    }

    /**
     * What to ask, or null when there is nothing to ask about.
     *
     * Null is not "no opinion" — it is the answer that means CLOSE IT NOW, and
     * the caller is expected to read it that way. Returning a question with an
     * empty body instead would put a dialog in front of every close, which is
     * exactly the tax the ruling refused.
     *
     * [now] is a parameter for `Terminal.displayDuration`'s reason: a clock read
     * inside is a value Compose cannot observe and nothing can test.
     */
    fun question(terminal: Terminal, now: Long): Question? {
        if (!mustAsk(terminal)) return null
        return Question(
            title = "Close “${terminal.label}”?",
            message = "${running(terminal, now)} Closing stops it and removes the tab. " +
                "There’s no undo.",
        )
    }

    /**
     * The first sentence: what is in this pane, and how long it has been at it.
     *
     * Three shapes, and the difference between them is what the runner has
     * actually said. `displayDuration` already picks the honest clock for each
     * state — the TURN's for a working agent, the STATE's for a blocked one —
     * and answers null both when the host never sent a timestamp and when the
     * answer would be under five seconds. Null there means the sentence loses
     * its clause rather than gaining a "0s", because "working for 0s" is the one
     * thing worse than not saying.
     */
    private fun running(terminal: Terminal, now: Long): String {
        val name = Terminal.name(terminal.preset)
        val doing = verb(terminal.agent)
        val elapsed = terminal.displayDuration(now)
        if (doing == null || elapsed == null) return "It’s running $name."
        return "It’s running $name, which has been $doing for $elapsed."
    }

    /**
     * How to say what the agent is doing, for the two states with a clock.
     *
     * Only `working` and `blocked`, and deliberately the same pair
     * `Terminal.statusDuration` answers for: an idle agent has been idle since
     * some moment nobody is interested in, and `done` is idle that nobody has
     * read. Neither is a thing you interrupt, so neither gets a clause claiming
     * it was.
     *
     * "Waiting on you" rather than `AgentActivity.label`'s "Needs you". The
     * label is a badge — a noun phrase for a chip — and this is the middle of a
     * sentence.
     */
    private fun verb(activity: AgentActivity): String? = when (activity) {
        AgentActivity.WORKING -> "working"
        AgentActivity.BLOCKED -> "waiting on you"
        AgentActivity.NONE, AgentActivity.IDLE, AgentActivity.DONE,
        AgentActivity.UNKNOWN -> null
    }
}
