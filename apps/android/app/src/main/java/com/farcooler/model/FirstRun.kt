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
    const val ONBOARDING_TITLE = "Connect a runner"
    const val ONBOARDING_BODY =
        "A runner is where your agents run: Far Cooler on a Mac or a Linux computer. " +
            "Connect this phone to one to see what they’re doing and answer them."
    const val ONBOARDING_PRIMARY = "Connect this device"
    const val ONBOARDING_SECONDARY = "Add a runner"
    const val NO_REPOSITORIES_BODY =
        "Add one here, or in the Mac app with File > Add Repository. Each one starts with a workspace called Main."
    const val ADD_REPOSITORY = "Add repository"
    const val NOTHING_NEEDS_YOU = "Nothing needs you"
    const val NO_ORCHESTRATOR_RUNNING = "No orchestrator is running yet. Open a workspace below to start one."
    const val ORCHESTRATOR_TITLE = "No orchestrator yet"
    const val ORCHESTRATOR_BODY =
        "Tell the orchestrator what you want done, and it plans the tasks and starts agents on them. " +
            "The first time, it asks a few questions about how you work."
    const val START = "Start orchestrator"

    /** Under a harness the runner doesn't have, which is disabled. */
    const val NOT_INSTALLED = "Not installed"
    const val TRY_AGAIN = "Try again"
    const val BOARD_TITLE = "No tasks"
    const val BOARD_NO_ORCHESTRATOR = "Start the orchestrator and tell it what you want done. Its tasks appear here."
    const val BOARD_WITH_ORCHESTRATOR = "Ask your orchestrator to plan the work. Its tasks appear here, grouped by status."
    const val SHOW_ORCHESTRATOR = "Show orchestrator"
    const val PUSH_BODY =
        "Notifications arrive only while Far Cooler is open. To get them when it’s closed, sign in, " +
            "then turn on notifications for your runner in Far Cooler on your Mac."
    const val SIGN_IN = "Sign in"

    /** The explainer before the permission request, after the first runner (iOS: `NotificationAsk`). */
    const val NOTIFY_TITLE = "Get notified when an agent needs you"
    const val NOTIFY_BODY = "Far Cooler can tell you when an agent asks a question or finishes."
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
            SHOW_ORCHESTRATOR, PUSH_BODY, SIGN_IN, NOTIFY_TITLE, NOTIFY_BODY, NOTIFY_ALLOW, NOTIFY_DECLINE,
            noRepositoriesTitle("build-01"),
        ) + AgentHarness.entries.flatMap { listOf(it.title, notInstalledTitle(it), notInstalledBody(it, "build-01")) }
}
