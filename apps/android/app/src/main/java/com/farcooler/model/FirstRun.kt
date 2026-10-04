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

/**
 * What Needs You says before there's anything to answer: a runner with no
 * repositories, and workspaces with no orchestrator running anywhere. The
 * iPhone's `PhoneFirstRun`, rule for rule.
 */
object PhoneFirstRun {
    /** Whether a runner whose fleet was read lists no repository at all. */
    fun hasNoRepositories(sections: List<RepositoryWorkspaces>): Boolean = sections.isEmpty()

    /**
     * Whether there are workspaces that could run an orchestrator and none
     * does, on any runner. An implicit workspace (a repository on a runner too
     * old for workspaces) can't have one, so it counts as neither.
     */
    fun noOrchestratorAnywhere(sections: List<RepositoryWorkspaces>): Boolean {
        val rows = sections.flatMap { it.workspaces }.filter { !it.workspace.isImplicit }
        return rows.isNotEmpty() && rows.all { it.orchestrator == null }
    }

    /** Whether a blank board offers Show orchestrator: one leads it and none is running. */
    fun offersOrchestrator(ledByOrchestrator: Boolean, orchestratorRunning: Boolean): Boolean =
        ledByOrchestrator && !orchestratorRunning

    /** What an empty board says under "No tasks". The iPhone's `BoardForm.blankCopy`. */
    fun blankCopy(ledByOrchestrator: Boolean, orchestratorRunning: Boolean): PhoneEmptyCopy = when {
        !ledByOrchestrator -> PhoneEmptyStates.BOARD_IMPLICIT
        orchestratorRunning -> PhoneEmptyStates.BOARD_WITH_ORCHESTRATOR
        else -> PhoneEmptyStates.BOARD_NO_ORCHESTRATOR
    }
}

/**
 * When the phone asks for notification permission: never before the person
 * has a runner, so the system's dialog doesn't cover the first screen, and a
 * refusal there can't be permanent. The iPhone's `NotificationAsk`.
 */
object NotificationAsk {
    /** Ask at launch only a phone that already has a runner: it has been through the explainer. */
    fun asksAtLaunch(hasRunners: Boolean): Boolean = hasRunners

    /** Whether the runner list going from [before] to [after] is the first runner arriving. */
    fun explainsAfterFirstRunner(hadRunners: Boolean, hasRunners: Boolean): Boolean = !hadRunners && hasRunners
}

/** The phone's first-run words, sentence case throughout. The iPhone's are `FirstRunCopy.Phone`. */
object FirstRunCopy {
    const val ONBOARDING_TITLE = "Connect to your agents"
    const val ONBOARDING_BODY =
        "Your agents work on a Mac or Linux computer that runs Far Cooler, called a runner. " +
            "Connect this phone to one to answer your agents while you’re away from it."
    const val ONBOARDING_PRIMARY = "Connect this device"
    const val ONBOARDING_SECONDARY = "Add a runner"
    const val ADD_REPOSITORY = "Add repository"
    const val NOTHING_NEEDS_YOU = "Nothing needs you"
    const val ORCHESTRATOR_TITLE = "No orchestrator yet"
    const val START = "Start orchestrator"

    /** Under a harness the runner doesn't have, which is disabled. */
    const val NOT_INSTALLED = "Not installed"
    const val TRY_AGAIN = "Try again"
    const val BOARD_TITLE = "No tasks"
    const val SHOW_ORCHESTRATOR = "Show orchestrator"
    const val PUSH_BODY =
        "To hear from your agents while Far Cooler is closed, sign in. Until then, notifications arrive only while it’s open."
    const val SIGN_IN = "Sign in"
    const val REPOSITORY_SUBTITLE = "Choose a Git repository on this runner for agents to work on."

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
            ONBOARDING_TITLE, ONBOARDING_BODY, ONBOARDING_PRIMARY, ONBOARDING_SECONDARY,
            ADD_REPOSITORY, NOTHING_NEEDS_YOU, ORCHESTRATOR_TITLE,
            START, NOT_INSTALLED, TRY_AGAIN, BOARD_TITLE,
            SHOW_ORCHESTRATOR, PUSH_BODY, SIGN_IN, REPOSITORY_SUBTITLE,
            NOTIFY_TITLE, NOTIFY_BODY, NOTIFY_ALLOW, NOTIFY_DECLINE,
            noRepositoriesTitle("build-01"),
        ) + PhoneEmptyStates.allStrings + AgentHarness.entries.flatMap { listOf(it.title, notInstalledTitle(it), notInstalledBody(it, "build-01")) }
}
