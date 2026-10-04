package com.farcooler.model

/**
 * First run (ov-205): which agents a runner can start, why an orchestrator's
 * pane died at once, and the words for a phone's empty states.
 *
 * A port of `AgentKit/FirstRun.swift` and the iPhone half of
 * `FirstRunCopy.swift`, rule for rule, in Android's sentence case.
 * `FirstRunTest` asserts what `FirstRunTests` does, so a change made on one
 * side and not the other fails on the side it was left out of.
 */

/**
 * A coding agent an orchestrator can run. [wire] is what `--harness` takes;
 * [program] is what the launch runs and what `Host.agents_found` names.
 */
enum class AgentHarness(val wire: String, val program: String, val title: String) {
    CLAUDE("claude", "claude", "Claude Code"),
    CODEX("codex", "codex", "Codex"),

    /** `cursor-agent`, not `cursor`, which is the editor. */
    CURSOR("cursor", "cursor-agent", "Cursor"),
    ;

    /** What a person installs to get [program]: installing Cursor, the editor, doesn't give you `cursor-agent`. */
    val installName: String get() = if (this == CURSOR) "the Cursor CLI" else title

    /** [installName] at the head of a title. */
    val installTitle: String get() = if (this == CURSOR) "Cursor CLI" else title
}

/**
 * Which harnesses a runner can start, read off `agentsFound`. Null is a runner
 * that doesn't say (no `agents_found` capability), which offers every harness:
 * its list is empty because it never sent one, not because nothing is there.
 */
data class HarnessAvailability(val agentsFound: List<String>?) {
    val isKnown: Boolean get() = agentsFound != null

    fun isInstalled(harness: AgentHarness): Boolean = agentsFound?.contains(harness.program) ?: true

    val installed: List<AgentHarness> get() = AgentHarness.entries.filter(::isInstalled)

    /** Empty on a runner that didn't say. */
    val missing: List<AgentHarness> get() = AgentHarness.entries.filterNot(::isInstalled)
}

/** Why an orchestrator's pane ended, when that's something the app can name. */
enum class OrchestratorExit {
    /** Exit status 127, POSIX's "command not found", from the pane's `-ilc` shell. */
    NOT_INSTALLED;

    companion object {
        /** How soon after starting a 127 still means "not installed". */
        const val WINDOW_MS: Long = 15_000

        /**
         * [NOT_INSTALLED] for a 127 within [WINDOW_MS] of the start, null
         * otherwise. The runner sends no time of exit, so [ranForMs] is the
         * app's own: from asking for the start to first seeing the exit.
         */
        fun classify(exitCode: Int?, ranForMs: Long): OrchestratorExit? =
            if (exitCode == 127 && ranForMs <= WINDOW_MS) NOT_INSTALLED else null
    }
}

/** The phone's first-run words, sentence case throughout. The iPhone's are `FirstRunCopy.Phone`. */
object FirstRunCopy {
    const val ONBOARDING_TITLE = "Connect to your agents"
    const val ONBOARDING_BODY =
        "Your agents work on a Mac or Linux computer that runs Far Cooler, called a runner. " +
            "Connect this phone to one to answer your agents while you’re away from it."
    const val ONBOARDING_PRIMARY = "Connect this device"
    const val ONBOARDING_SECONDARY = "Add a runner"
    const val NO_REPOSITORIES_BODY =
        "Add the Git repository you want agents to work on, here or on your Mac with File > Add Repository."
    const val ADD_REPOSITORY = "Add repository"
    const val NOTHING_NEEDS_YOU = "Nothing needs you"
    const val NO_ORCHESTRATOR_RUNNING =
        "No agents are working yet. Each workspace below is one line of work, like a feature or a cleanup. " +
            "Open one and start its orchestrator to give agents something to do."
    const val ORCHESTRATOR_TITLE = "No orchestrator yet"
    const val ORCHESTRATOR_BODY =
        "Instead of running each agent yourself, tell the orchestrator what you want done. " +
            "It splits the work into tasks and starts an agent on each. The first time, it asks how you like to work."
    const val START = "Start orchestrator"

    /** Under a harness the runner doesn't have, which is disabled. */
    const val NOT_INSTALLED = "Not installed"
    const val TRY_AGAIN = "Try again"
    const val BOARD_TITLE = "No tasks"
    const val BOARD_NO_ORCHESTRATOR =
        "Start the orchestrator and tell it what you want done. Each piece of work it hands out appears here as a task."
    const val BOARD_WITH_ORCHESTRATOR =
        "Tell the orchestrator what you want done. Each piece of work it hands out appears here as a task."
    const val SHOW_ORCHESTRATOR = "Show orchestrator"
    const val PUSH_BODY =
        "To hear from your agents while Far Cooler is closed, sign in. Until then, notifications arrive only while it’s open."
    const val SIGN_IN = "Sign in"
    const val WORKTREES_NONE =
        "No worktrees yet. When the orchestrator starts an agent on a task, the agent gets its own folder and branch, listed here."
    const val REPOSITORY_SUBTITLE = "Choose a Git repository on this runner for agents to work on."
    const val REPOSITORY_SUBTITLE_FOR_WORKTREE = "Choose the repository for the new worktree."
    const val BOARD_IMPLICIT = "Each piece of work on this board appears here as a task."

    /** The explainer before the permission request, after the first runner (iOS: `NotificationAsk`). */
    const val NOTIFY_TITLE = "Get notified when an agent needs you"
    const val NOTIFY_BODY =
        "Your agents keep working without you. Far Cooler can tell you when one has a question or finishes, " +
            "so you don’t have to keep checking."
    const val NOTIFY_ALLOW = "Allow notifications"
    const val NOTIFY_DECLINE = "Not now"

    fun noRepositoriesTitle(runner: String): String = "No repositories on $runner"

    fun notInstalledTitle(harness: AgentHarness): String = "${harness.installTitle} isn’t installed"

    fun notInstalledBody(harness: AgentHarness, runner: String): String =
        "There’s no ${harness.program} command on $runner. Install ${harness.installName} there, then try again."

    /** Every string above, each function at every harness, for the voice check. */
    internal val all: List<String>
        get() = listOf(
            ONBOARDING_TITLE, ONBOARDING_BODY, ONBOARDING_PRIMARY, ONBOARDING_SECONDARY, NO_REPOSITORIES_BODY,
            ADD_REPOSITORY, NOTHING_NEEDS_YOU, NO_ORCHESTRATOR_RUNNING, ORCHESTRATOR_TITLE, ORCHESTRATOR_BODY,
            START, NOT_INSTALLED, TRY_AGAIN, BOARD_TITLE, BOARD_NO_ORCHESTRATOR, BOARD_WITH_ORCHESTRATOR,
            SHOW_ORCHESTRATOR, PUSH_BODY, SIGN_IN, WORKTREES_NONE, REPOSITORY_SUBTITLE,
            REPOSITORY_SUBTITLE_FOR_WORKTREE, BOARD_IMPLICIT, NOTIFY_TITLE, NOTIFY_BODY, NOTIFY_ALLOW, NOTIFY_DECLINE,
            noRepositoriesTitle("build-01"),
        ) + AgentHarness.entries.flatMap { listOf(it.title, notInstalledTitle(it), notInstalledBody(it, "build-01")) }
}
