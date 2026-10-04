package com.farcooler.model

/**
 * What a terminal with no running pane says, and the two ways off it (ov-191).
 *
 * **A port of `AgentKit/LostPane.swift`, sentence for sentence**, which the
 * Mac's lost-terminal page and the iPhone's pane both read. Both sides'
 * tests replay `test/fixtures/lost-pane.json` and compare whole strings, so a
 * sentence changed on one side and not the other fails on the side left
 * behind.
 *
 * Restart runs the terminal's preset again: an agent comes back as that agent,
 * and a shell as a shell. What was typed into a shell was never recorded, so a
 * shell's note says so before Restart is pressed.
 */
object LostPane {
    enum class Kind { LOST, EXITED, ERROR }

    /**
     * The two answers, each with its one label and the sentence a refusal
     * reads as. One word each, so Material's sentence case and Apple's title
     * case agree.
     */
    enum class Action(val title: String, val failure: String) {
        RESTART("Restart", "Couldn’t restart this terminal."),
        DISMISS("Dismiss", "Couldn’t dismiss this terminal."),
    }

    /** The page's kind for a terminal state, or null for a pane that's running or may be. */
    fun kind(state: StateKind): Kind? = when (state) {
        StateKind.LOST -> Kind.LOST
        StateKind.EXITED -> Kind.EXITED
        StateKind.ERROR -> Kind.ERROR
        StateKind.STARTING, StateKind.RUNNING, StateKind.UNKNOWN -> null
    }

    /** Dismiss for lost alone: the one state `terminal.dismiss_lost` accepts. */
    fun actions(kind: Kind): List<Action> = when (kind) {
        Kind.LOST -> listOf(Action.RESTART, Action.DISMISS)
        Kind.EXITED, Kind.ERROR -> listOf(Action.RESTART)
    }

    fun title(kind: Kind): String = when (kind) {
        Kind.LOST -> "Terminal Lost" // casing ok: pinned to test/fixtures/lost-pane.json, shared with Apple
        Kind.EXITED -> "Terminal Ended" // casing ok: pinned to the shared fixture
        Kind.ERROR -> "Terminal Didn’t Start" // casing ok: pinned to the shared fixture
    }

    fun explanation(kind: Kind): String = when (kind) {
        Kind.LOST ->
            "Its tmux pane is gone. The pane was closed outside Far Cooler, tmux was quit, " +
                "or the runner restarted. Nothing is running in it now."
        Kind.EXITED -> "Its program ended. Nothing is running in it now."
        Kind.ERROR -> "The runner couldn’t start it."
    }

    /** What Restart brings back, said before it's pressed. An empty preset is a shell. */
    fun restartNote(preset: String): String = when (val program = preset.substringBefore(':')) {
        "", "shell" ->
            "Restart opens a new shell in this worktree. What was running in it wasn’t " +
                "recorded, so you’ll need to start that again."
        "claude" -> "Restart opens Claude Code again in this worktree, back in its conversation if it had one."
        "codex" -> "Restart opens Codex again in this worktree, back in its conversation if it had one."
        "cursor" -> "Restart opens Cursor again in this worktree."
        else -> "Restart runs $program again in this worktree."
    }

    const val DISMISS_NOTE = "Dismiss removes it from the list."

    /** The whole sentence a phone's pane says under the title: why, what Restart brings back, and Dismiss when offered. */
    fun message(kind: Kind, preset: String): String =
        listOfNotNull(
            explanation(kind),
            restartNote(preset),
            DISMISS_NOTE.takeIf { Action.DISMISS in actions(kind) },
        ).joinToString(" ")
}
