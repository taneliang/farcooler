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
    /**
     * Which repository this worktree belongs to, as a UUID string.
     *
     * Nullable because an older daemon's fleet never carried it, and one missing
     * field must not fail the decode of the whole fleet. Everything
     * repository-scoped a client can ask about a workspace — its stack, its pull
     * request, the repository's display name — needs this, and none of those
     * screens exist here yet: decoded now so the one phase allowed inside this
     * file does not have to be reopened by the phase that needs it.
     */
    val repository: String? = null,
    val task: String = "",
    val branch: String = "",
    val worktree: String? = null,
    val state: String = "",
    /**
     * Whether this workspace IS the repository's own checkout.
     *
     * Offering to remove it would offer to delete the directory the repository
     * itself lives in, and the branch is not the useful fact about it — the Mac
     * writes "Primary checkout" where it would otherwise write a branch name.
     *
     * Sent as `isMainCheckout` by the client core —
     * `crates/client/src/session.rs:285` — which is NOT what the Mac reads: the
     * CLI at `crates/cli/src/main.rs:1575` spells the same flag
     * `is_main_checkout`, and the Mac's model spells its property to match. iOS
     * copied the Mac's property name onto the client core's payload, so
     * `Workspace.isMainCheckout` there decodes nothing and is false for every
     * workspace on the phone — "Primary checkout" never appears and the remove
     * offer is made on the one worktree it must never be made on. Recorded here
     * rather than fixed, because iOS is not this agent's to edit.
     */
    val isMainCheckout: Boolean = false,
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

    /**
     * git no longer lists this worktree, but the row still carries terminals.
     *
     * A state of the WORKSPACE and not of any pane in it, which is why the other
     * two surfaces say it once on the header rather than letting twenty
     * terminals fail separately underneath. No screen here says it yet.
     */
    val worktreeMissing: Boolean get() = state.equals("worktree_missing", ignoreCase = true)
}

@Serializable
data class Terminal(
    val id: String,
    val short: String = "",
    val title: String = "",
    val preset: String = "",
    val state: String = "",
    /**
     * How the process ENDED: the code it exited with, and the signal that
     * killed it.
     *
     * Both have been on the wire beside [state] all along —
     * `crates/client/src/session.rs:298-299` — and both were dropped on the way
     * in here, so an `exited` row on this phone said only that the process was
     * gone. A shell somebody closed and a `cargo build` that broke are the same
     * word to [state], and they are not the same news.
     *
     * Nullable for the reason every field added to this type is nullable: a
     * daemon built before exit status existed sends no key, and a key this
     * decoder required would fail the WHOLE fleet rather than cost one row its
     * ending. Absent means "nobody said", never "it exited cleanly" — see
     * [runDidFail], which refuses to read one as the other.
     */
    val exitCode: Int? = null,
    val exitSignal: Int? = null,
    /**
     * What the agent is doing, derived on the HOST. A phone has no screen to
     * inspect, so this arriving over the wire is the only way it can know — and
     * it is why the same badge means the same thing here as on the Mac.
     */
    val activity: String? = null,
    /**
     * Whether the turn the agent just finished DIED rather than completed.
     *
     * Read from the agent's own session log on the host, and carried beside
     * [activity] because that has no word for it: a turn that died and one that
     * succeeded are both `done` there. This app decoded no such field, so it
     * drew the green checkmark of a finished turn over one that had stopped
     * working — see `attentionColor(Terminal)`. Sent as `turnFailed`; see
     * `crates/cli/src/main.rs` and iOS's `Terminal.turnFailed`.
     *
     * Nullable rather than defaulted to false, on the rule every field added to
     * this type follows: a daemon built before it sends no key at all, and
     * absent means "nothing claimed the turn went badly" rather than a real
     * answer.
     */
    val turnFailed: Boolean? = null,
    /**
     * Unix milliseconds when the current [activity] began, or null when the host
     * did not say.
     *
     * Distinct from [turnStartedAt]: this restarts whenever the state changes,
     * so it answers "how long has this been blocked" rather than "how long has
     * this turn been running". Timed on the HOST rather than here, because a
     * clock started on the phone restarts at every reconnect and lies across a
     * laptop sleep.
     *
     * A [Double] because that is what a JSON number of milliseconds decodes to
     * without a range to worry about, and the only arithmetic done with it is a
     * subtraction against the wall clock — see [displayDuration].
     */
    val activitySince: Double? = null,
    /**
     * Unix milliseconds when the current turn started, or null between turns.
     *
     * Held across Blocked on the host: approving a tool call does not begin a
     * new turn, so a row's clock does not restart when you answer one.
     */
    val turnStartedAt: Double? = null,
    /** What the agent is asking, while it is asking it. */
    val blockedQuestion: String? = null,
    /**
     * The last few things the agent SAID, oldest first, at most three.
     *
     * A transcript and only a transcript — the agent's own prose, with no verb
     * in front of it. What it DID arrives on [line] instead. Already redacted
     * and cut to a row's width by the daemon, so this app renders them and
     * decides nothing about them.
     */
    val feed: List<String>? = null,
    /**
     * The last thing the agent said, WHOLE and from its opening.
     *
     * The same message [feed]'s last lines were cut from, cut from the other end
     * and to a notification's width rather than a row's — and a separate field
     * because it cannot be recovered from those lines: a feed entry is a wrapped
     * ROW, so the last of them is the last forty characters of the window. That
     * is how a lock screen came to read "batches to avoid N+1 shits." about a
     * turn that had ended "More shit. An industrial quantity of shit, shipped in
     * carefully authorized batches to avoid N+1 shits."
     *
     * Cut on the host to about 120 characters; see
     * `farcooler_core::feed::SAID_WIDTH`. Absent means "ask the feed instead"
     * rather than "nothing was said" — see [lastSaid].
     */
    val said: String? = null,
    /**
     * The agents this agent spawned and has not finished with, named. Their
     * COUNT is already inside [line]; these are the names.
     */
    val subagents: List<String>? = null,
    /**
     * The state in one character: `?` blocked, `●` working, `✓` done, `✗`
     * failed, `·` idle. The narrowest rung of the host's compact ladder.
     *
     * Decoded and, for now, read by nothing. This app has no widget, no watch
     * face and no ongoing notification yet — the three surfaces the narrow rungs
     * exist for. It is here because the whole ladder travels together
     * deliberately: a surface that re-derives one rung from a wider one is a
     * second derivation of a fact the host derives once, and that is exactly
     * what having it already decoded prevents the next reader from doing.
     */
    val glyph: String? = null,
    /** The state plus just enough to say whose, at most ~18 characters. */
    val headline: String? = null,
    /**
     * Where the agent is, in one line: the question it is blocked on, its
     * position in its own task list, or what it is doing right now.
     *
     * One rung of the daemon's compact ladder. The priority between those three
     * is decided on the host — see `farcooler_core::feed::line` — because a Mac,
     * a phone and a watch deciding it separately is three surfaces disagreeing
     * about one pane.
     */
    val line: String? = null,
    /**
     * Where this terminal sorts in a fleet view. SMALLER sorts FIRST: blocked
     * outranks done outranks working, and within a tier the oldest first.
     *
     * Computed on the host beside [activity] — `farcooler_core::feed::rank` — so
     * a notification about one agent and a list showing twelve agree about which
     * one matters. Nothing on this side may re-derive an ordering; that is the
     * whole reason the field exists.
     *
     * **This app does not sort its fleet list by it, and that is deliberate.**
     * See the comment above the `items(...)` call in `FleetScreen`: rows that
     * re-order themselves as agents finish slide the row you had already
     * committed to tapping out from under your thumb. Attention is a mark on a
     * row here. Rank is for CHOOSING one pane out of many — which agent a
     * notification is about, which row a front door leads with — not for moving
     * rows around.
     *
     * A [Long] where the wire says `uint32`. The values in practice top out
     * around 4×10⁸ so an [Int] would hold them, but a `uint32` above
     * [Int.MAX_VALUE] would throw mid-decode and take the WHOLE fleet down with
     * it. The one field on this type whose range is not obviously safe is not
     * the place to save four bytes.
     */
    val rank: Long? = null,
    /**
     * How far the agent is through its OWN task list: [planDone] of 4 and
     * [planTotal] of 7 is `4/7`.
     *
     * The same position [line] may already state in words, carried as the
     * numbers it was composed from. Separate fields rather than something read
     * back out of that string, because [line] is a RUNG: the question outranks
     * the task count, so a blocked agent's line is the question and holds no
     * numbers at all.
     *
     * Null is not zero. Null is "the host said nothing about a task list" — a
     * daemon too old to send these, a pane with no session log, an agent that
     * never wrote a list, and every codex and cursor pane. `0` of `7` is a
     * written list with nothing finished, which is a different thing and reads
     * differently.
     */
    val planDone: Int? = null,
    val planTotal: Int? = null,
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

    /** Whether the last turn ended badly. Only `done` can answer this. */
    val turnDidFail: Boolean get() = agent == AgentActivity.DONE && turnFailed == true

    /** What this agent's state is called in a row, failure included. */
    val activityLabel: String get() = if (turnDidFail) "Failed" else agent.label

    /**
     * Whether the PROCESS ended badly, as opposed to the turn that ran inside
     * it.
     *
     * The companion to [turnDidFail], and deliberately a separate question:
     * that one is about the agent's last turn, read from its session log; this
     * one is about the command, read from how its process exited. A `cargo
     * build` that returned 101 has no turns at all.
     *
     * The Mac's rule, verbatim, and iOS's: a signal or a non-zero code is a
     * failure worth seeing; a clean exit is not; and an ABSENT code is not a
     * failure either. That last clause is the one that matters, because an older
     * daemon sends no exit status at all, and reading nothing as broken would
     * mark every finished terminal on the runner as failed.
     *
     * Gated on `exited` on the same terms the other two surfaces gate it, so the
     * three cannot disagree about which terminals ended badly: [state] is the
     * daemon's word for whether the process is gone, and how it ended is a
     * question only a process that HAS ended can answer.
     */
    val runDidFail: Boolean
        get() {
            if (StateKind.parse(state) != StateKind.EXITED) return false
            return exitSignal != null || (exitCode?.let { it != 0 } ?: false)
        }

    /**
     * The signal line, or empty when the host has nothing to say.
     *
     * Trimmed here rather than at each call site: a line that is whitespace is a
     * line that draws a blank row and makes every surface taller for nothing,
     * and three surfaces trimming it separately is three chances to forget.
     */
    val signalLine: String get() = (line ?: "").trim()

    /**
     * The last few things the agent said, trimmed and capped at three.
     *
     * The cap is repeated here even though the daemon already keeps only three,
     * so a host that ever sent four could not make one row twice the height of
     * every other row in the list.
     *
     * Kept when the agent goes idle rather than cleared. "What did this do while
     * I was away" is exactly when the summary is worth most, and a row that shed
     * its lines on going idle would also mean the list rearranging itself under
     * somebody reading it.
     */
    val recentSteps: List<String>
        get() = (feed ?: emptyList()).map { it.trim() }.filter { it.isNotEmpty() }.takeLast(3)

    /**
     * What to quote in a notification about this pane.
     *
     * [said] and NOT `recentSteps.last()`. The two are cut from one message at
     * opposite ends — a step is a wrapped row, so the last of them is the end of
     * the window, while a notification arrives after the fact and has to open
     * where the sentence opens. The cut is the host's; see
     * `farcooler_core::feed::Feed::said`.
     *
     * The feed's last line is the fallback and only that: a runner still on an
     * older daemon sends no [said], and the tail of the window is a worse
     * sentence than the head but a much better one than nothing.
     */
    val lastSaid: String?
        get() {
            val quoted = (said ?: "").trim()
            if (quoted.isNotEmpty()) return quoted
            return recentSteps.lastOrNull()
        }

    /**
     * The subagents still running, named, at most three.
     *
     * Their COUNT is already inside [line]; these are the names, and three is
     * what fits beside a row on a phone.
     */
    val runningSubagents: List<String>
        get() = (subagents ?: emptyList()).filter { it.isBlank().not() }.take(3)

    /**
     * Whether anything in this row changes with the passing of time.
     *
     * Asked BEFORE the duration rather than derived from it, because a row's
     * ticker has to be started or not started, and the answer to "is there a
     * clock here" must not depend on what the clock currently reads: a working
     * agent four seconds in has no duration string yet — see [brief] — and a row
     * that read `displayDuration == null` as "no clock" would never start
     * ticking and would never show one.
     */
    val hasClock: Boolean
        get() = agent == AgentActivity.WORKING || agent == AgentActivity.BLOCKED

    /**
     * How long the current state has been the state, for the two states where
     * the answer changes what you do.
     *
     * An agent blocked for twenty minutes is a different situation from one
     * blocked for ten seconds. "Idle for three days" is noise, so it is null.
     */
    fun statusDuration(now: Long): String? {
        if (!hasClock) return null
        val since = activitySince ?: return null
        return brief(since, now)
    }

    /**
     * How long the whole turn has run. Does not restart when a permission prompt
     * is approved, because saying yes to a tool call does not begin a new turn.
     */
    fun turnDuration(now: Long): String? {
        val since = turnStartedAt ?: return null
        return brief(since, now)
    }

    /**
     * The one duration worth putting beside the status label.
     *
     * The two clocks answer different questions and conflating them is the bug
     * they exist to fix. `Working` is only ever mid-turn, so the TURN clock is
     * the honest answer to "how long has this been going". `Blocked` wants the
     * STATE clock, because a prompt held for twenty minutes is the thing to
     * notice, not how long the turn around it has run.
     *
     * [now] is an ARGUMENT rather than a `System.currentTimeMillis()` read
     * inside, and that is what makes the string tick. Read inside, it is a value
     * Compose cannot observe: nothing about a working row changes from one
     * recomposition to the next, so the row is never invalidated and the
     * duration freezes until something unrelated redraws it. Taken as an
     * argument it is an input like any other, and one `State<Long>` advancing
     * once a second is a row that keeps its own time.
     */
    fun displayDuration(now: Long): String? =
        if (agent == AgentActivity.WORKING) turnDuration(now) else statusDuration(now)

    /**
     * What a fleet row says about this pane's state, beside its name.
     *
     * "Working 12m", "Needs you 2m" — and for a pane that is not an agent, the
     * process state, or nothing at all when it is simply running. That silence
     * is the point rather than a fact withheld: `ProcessDot` draws nothing for a
     * running process either, so a live shell is a row with a name on it, and
     * the word "running" under an agent's name was the string this row spent its
     * most valuable line on while restating the dot beside it.
     *
     * Null under five seconds of elapsed time as well, via [displayDuration], so
     * a row does not flicker "1s" on its way to saying something useful — the
     * label is shown alone until there is an age worth printing.
     *
     * Here rather than in the row that draws it, beside [activityLabel], which
     * is the same kind of thing: it is a pure function of a decoded pane and a
     * moment, which is exactly what a unit test can pin and a Compose preview
     * cannot.
     */
    fun rowStatus(now: Long): String? {
        if (!agent.isAgent || agent == AgentActivity.UNKNOWN) {
            if (StateKind.parse(state) == StateKind.RUNNING) return null
            // Blank only if a daemon sent no state at all; an empty string would
            // draw an empty line with padding around it.
            return state.lowercase().ifBlank { null }
        }
        val elapsed = displayDuration(now) ?: return activityLabel
        return "$activityLabel $elapsed"
    }

    /**
     * Where this terminal sorts. An absent [rank] sorts LAST: a daemon too old
     * to send one is a daemon that cannot tell us this pane is urgent, and
     * guessing that it is would put an unknown above a known blocked agent.
     */
    val sortRank: Long get() = rank ?: Long.MAX_VALUE

    /** Whether to draw a chat or a VT grid. */
    val isAgentPane: Boolean get() = paneMode == "agent"

    /**
     * Whether this pane is a review of what its worktree changed.
     *
     * The daemon has served this mode since the review surface landed and this
     * app has no branch for it: a `changes` pane falls past [isAgentPane] to the
     * VT renderer at `TerminalScreen.kt:334` and is drawn as a raw terminal — a
     * grid of whatever bytes are on a pane that is not a tty. Decoded here so
     * the screen that has to route around it can ask; the routing itself is a
     * later phase's, and until it lands this property has no caller.
     */
    val isChangesPane: Boolean get() = paneMode == "changes"

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
        /**
         * An elapsed time in one or two characters, or null when there is not
         * yet anything worth saying.
         *
         * Under five seconds is null so a row does not flicker "1s" on its way
         * to saying something useful, and a clock that has somehow run backwards
         * — a runner whose wall clock is ahead of this phone's — falls in the
         * same branch rather than printing a negative age.
         *
         * The same three units iOS and the Mac print, in the same order and with
         * the same truncation, because one person reads "12m" on a laptop and on
         * a phone about one pane.
         */
        fun brief(sinceMillis: Double, now: Long): String? {
            val seconds = (now - sinceMillis) / 1000
            if (seconds < 5) return null
            if (seconds < 60) return "${seconds.toInt()}s"
            if (seconds < 3600) return "${(seconds / 60).toInt()}m"
            return "${(seconds / 3600).toInt()}h"
        }

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
 * The terminal a host lands on when its workspace list is skipped.
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

/**
 * What went wrong, in two voices.
 *
 * [sentence] is Far Cooler talking: what happened, and — where this side can
 * know one — what to do about it. [transcript] is what the runner, the client
 * core or the platform said back, and the two are never joined. A runner's
 * words spliced onto the app's with a colon read as the app's own account of a
 * machine it cannot see, which is the shape `776d3e0`, `e0f72df` and `c42c352`
 * removed from the Mac and the phone.
 *
 * Nothing is discarded by keeping them apart. For a runner nobody can reach,
 * that text is the only diagnosis there is; it goes in a `DetailBox`, where
 * output goes, rather than where prose does.
 *
 * Null where there is nothing to show, so a screen can ask before it reserves
 * the space — an empty box under a sentence reads as output that failed to
 * arrive.
 *
 * One type, not one per surface. The Apple apps spell this idea four times —
 * `AgentStream.Trouble`, `ChangesStore.Trouble`, `SheetFailure` and
 * `TerminalSession.Phase.failed` — because a framework boundary stands between
 * some of them. Nothing stands between these screens.
 */
data class Trouble(val sentence: String, val transcript: String? = null)
