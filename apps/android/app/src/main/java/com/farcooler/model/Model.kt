package com.farcooler.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

// The shapes the client core returns. Identical to the Mac and iOS apps',
// because all three decode what one Rust crate produces — there is one
// definition of what a workspace looks like on the wire, not one per platform.

@Serializable
data class Fleet(
    @SerialName("runtime_healthy") val runtimeHealthy: Boolean = false,
    @SerialName("live_panes") val livePanes: Int = 0,
    val workspaces: List<Workspace> = emptyList(),
) {
    companion object {
        val EMPTY = Fleet()
    }
}

@Serializable
data class Workspace(
    val id: String,
    val short: String = "",
    val task: String = "",
    val branch: String = "",
    val worktree: String? = null,
    val state: String = "",
    val terminals: List<Terminal> = emptyList(),
) {
    /**
     * Which of several identically-labelled terminals each one is, keyed by
     * terminal id.
     *
     * Two `claude` panes in one workspace are genuinely alike, so they get `1`
     * and `2` — but only when there is something to tell apart, or a lone
     * `shell` would be numbered for no reason. Shared by the fleet list, the
     * terminal screen's title and its tab strip, so the same terminal is never
     * numbered differently depending on which screen is showing it.
     */
    fun ordinals(): Map<String, Int> {
        val counts = terminals.groupingBy { it.label }.eachCount()
        val seen = mutableMapOf<String, Int>()
        val out = mutableMapOf<String, Int>()
        for (terminal in terminals) {
            if ((counts[terminal.label] ?: 0) <= 1) continue
            val next = (seen[terminal.label] ?: 0) + 1
            seen[terminal.label] = next
            out[terminal.id] = next
        }
        return out
    }

    /** Hidden workspaces are a view preference the daemon records for us. */
    val isHidden: Boolean get() = state.equals("hidden", ignoreCase = true)
}

@Serializable
data class Terminal(
    val id: String,
    val short: String = "",
    val title: String = "",
    val preset: String = "",
    val state: String = "",
    /**
     * What the agent is doing, derived on the HOST. A phone has no screen to
     * inspect, so this arriving over the wire is the only way it can know — and
     * it is why the same badge means the same thing here as on the Mac.
     */
    val activity: String? = null,
    val epoch: Int = 0,
    /**
     * What this terminal's pane is hosting. Absent on older daemons, which is
     * why it is nullable rather than defaulted to something that would look
     * like a real answer.
     */
    val paneMode: String? = null,
    val chatCapable: Boolean? = null,
    val agentSessionId: String? = null,
    val agentMode: String? = null,
    val availableAgentModes: List<String>? = null,
) {
    val agent: AgentActivity get() = AgentActivity.parse(activity)

    /** Whether to draw a chat or a VT grid. */
    val isAgentPane: Boolean get() = paneMode == "agent"

    /**
     * Whether this pane can be shown as a chat.
     *
     * Answered on the host, because identifying an agent takes a screen read —
     * Claude Code renames its own process. Absent from older daemons, and
     * absent means "do not offer": a switch that came back as a different agent
     * is worse than no switch at all.
     */
    val canSwitchPaneMode: Boolean get() = chatCapable == true

    /**
     * What to call this terminal.
     *
     * Derived, never stored, because a terminal IS the thing running in it.
     * [preset] already carries what tmux reports is running — `claude`,
     * `codex`, `zsh` — resolved on the host, because only the host has a screen
     * to look at.
     */
    val label: String
        get() {
            // The conversation's own name, when the agent has given it one.
            if (title.isNotEmpty() && !isPlaceholder(title)) return title
            return name(preset)
        }

    /** `label`, plus its ordinal when it has one. */
    fun displayName(ordinal: Int?): String {
        // A named conversation needs no counter: the ordinal exists to tell
        // three identical "claude"s apart, and a title already has.
        if (ordinal == null || label != name(preset)) return label
        return "$label $ordinal"
    }

    companion object {
        /** Whether a title is the automatic one every terminal is created with. */
        private fun isPlaceholder(title: String) =
            title.startsWith("Terminal ") || title == "Terminal"

        /** One word for one thing, wherever a running command is shown. */
        fun name(command: String): String {
            val running = command.trim().lowercase()
            if (running.isEmpty()) return "shell"
            // The host reports whatever tmux sees running, so the same plain
            // shell arrives as `zsh` from a pane the watcher has looked at and
            // as `shell` from one it has not. Normalising both to `shell` is
            // what keeps two identical shells from reading as different things.
            return if (running in SHELLS) "shell" else running
        }

        private val SHELLS = setOf("sh", "zsh", "bash", "fish", "dash", "ksh", "-zsh")
    }
}

/**
 * What a coding agent is doing, as distinct from whether its process is alive.
 *
 * `DONE` is idle that nobody has looked at yet — which is what makes it the
 * thing worth a notification, and what makes it clear itself when you open the
 * terminal.
 */
enum class AgentActivity(val wire: String) {
    NONE("none"),
    IDLE("idle"),
    WORKING("working"),
    BLOCKED("blocked"),
    DONE("done"),
    UNKNOWN("unknown");

    /** The single definition of "interrupt someone", shared with the Mac. */
    val wantsAttention: Boolean get() = this == BLOCKED || this == DONE

    val isAgent: Boolean get() = this != NONE

    val label: String
        get() = when (this) {
            NONE -> ""
            IDLE -> "Idle"
            WORKING -> "Working"
            BLOCKED -> "Needs you"
            DONE -> "Done"
            UNKNOWN -> "Unknown"
        }

    companion object {
        fun parse(raw: String?): AgentActivity {
            if (raw == null) return NONE
            return entries.firstOrNull { it.wire == raw } ?: UNKNOWN
        }
    }
}

/** The states a terminal can be in, grouped by what a user should do about it. */
enum class StateKind {
    STARTING,
    RUNNING,
    EXITED,
    ERROR,
    LOST,
    UNKNOWN;

    /**
     * Lost is red because it is the one state that means Far Cooler does not
     * know what happened, and the user has to decide.
     */
    val isAttentionWorthy: Boolean get() = this == LOST || this == ERROR

    companion object {
        fun parse(raw: String): StateKind = when (raw.lowercase()) {
            "starting" -> STARTING
            "running" -> RUNNING
            "exited" -> EXITED
            "error" -> ERROR
            "lost" -> LOST
            else -> UNKNOWN
        }
    }
}

@Serializable
data class Repository(
    val id: String,
    val short: String = "",
    val displayName: String = "",
    val remote: String = "",
)

@Serializable
data class RepositoryList(val repositories: List<Repository> = emptyList())

/** What the daemon on the other end is, asked once per connection. */
/**
 * What the daemon on the other end said about itself.
 *
 * [capabilities] is distinct from [matches], and they answer different
 * questions. That one is "were these built from the same source"; this is "what
 * can that runner do", which is the one an app acts on when it is newer than
 * the runner it reached — Play review and `runner install` do not tick together.
 */
data class DaemonBuild(
    val version: String,
    val matches: Boolean,
    val platform: String,
    val capabilities: Set<String> = emptySet(),
) {
    /**
     * Whether this runner can do something, by name.
     *
     * A control whose capability is missing is shown DIMMED with a reason,
     * never hidden: the same app showing different controls on two runners
     * with nothing said about why reads as a bug.
     *
     * An empty set means a daemon old enough to predate the question, so it has
     * exactly the feature set that existed then. Reading silence as "can do
     * nothing" would blank the UI against every older runner, which is the
     * opposite of the point. Matches the iOS reading exactly.
     */
    fun can(capability: String): Boolean {
        if (capabilities.isEmpty()) return capability == "workspaces" || capability == "terminals"
        return capabilities.contains(capability)
    }
}

/**
 * The terminal a host lands on when its worktree list is skipped.
 *
 * An agent waiting on you outranks everything else, because that is the whole
 * reason to have opened the app; short of that, the first terminal already
 * running is a better first screen than an arbitrary one that has exited or
 * never started. Null only when the host has no terminals at all.
 */
val Fleet.landingTerminal: Terminal?
    get() {
        val all = workspaces.flatMap { it.terminals }
        all.firstOrNull { it.agent.wantsAttention }?.let { return it }
        all.firstOrNull { StateKind.parse(it.state) == StateKind.RUNNING }?.let { return it }
        return all.firstOrNull()
    }
