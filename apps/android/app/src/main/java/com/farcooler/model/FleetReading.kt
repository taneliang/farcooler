package com.farcooler.model

/**
 * How far a runner's last fleet can be believed, from how its link stands.
 *
 * Three answers, because "is this runner's fleet current" has a third honest
 * reply besides yes and no: nothing has answered yet. The net layer's
 * `Connection.Phase` maps onto this, so the rules below can be tested without a
 * connection.
 */
enum class RunnerLink {
    /** Still on its first read. Nothing it has said is on screen yet. */
    CONNECTING,

    /** Connected right now. What it last said is what it says. */
    ANSWERING,

    /**
     * Reconnecting, failed or waiting on a person. Its last fleet stays on
     * screen so the list doesn't move, and the agents in it may have exited
     * since. A runner that stays down spends most of its outage here,
     * reconnecting between attempts.
     */
    AWAY,
}

/** Which link read the fleet a runner's rows are drawn from. See [given]. */
enum class FleetRead {
    /** No link has read one: the rows are empty. */
    NEVER,

    /** A link before this one did, and this one has not yet. */
    EARLIER_LINK,

    /** The link that is up now. */
    THIS_LINK,

    /**
     * No link has read one, and this link's read failed without the link
     * dropping: a reply the app could not decode, say. Away ("can't say")
     * rather than connecting, which it would otherwise read as for as long as
     * every poll failed the same way.
     */
    FAILED;

    /** After a fleet read failed without the link dropping. Only a first read changes anything. */
    fun failed(): FleetRead = if (this == NEVER) FAILED else this

    /** A new link: whatever was read, it was read on an earlier one. */
    fun onNewLink(): FleetRead = when (this) {
        THIS_LINK -> EARLIER_LINK
        FAILED -> NEVER
        else -> this
    }
}

/**
 * This link, weakened by what its fleet can vouch for.
 *
 * A reconnect is Connected for a host read and a fleet read before it has
 * heard anything, and until then the rows on screen are the last link's:
 * agents that may have exited since. So a connected runner answers only once
 * this link has read its fleet — the iPhone's `hasFleet`. Before that it is
 * away, with the old rows, or on a first link, which has none, connecting.
 */
fun RunnerLink.given(read: FleetRead): RunnerLink = when {
    this != RunnerLink.ANSWERING || read == FleetRead.THIS_LINK -> this
    read == FleetRead.NEVER -> RunnerLink.CONNECTING
    // EARLIER_LINK, with the old rows, and FAILED, whose read could not be
    // made sense of: neither vouches for now.
    else -> RunnerLink.AWAY
}

/**
 * A pane's process state, as far as its runner can vouch for it: what it
 * last said while it answers, and "can't say" — the hollow neutral dot —
 * while it doesn't. An exited gray dot or a lost red ring from a fleet read
 * before the link went is a claim about now nobody made.
 */
fun StateKind.said(answering: Boolean): StateKind = if (answering) this else StateKind.UNKNOWN

/**
 * What a surface may say about the fleet's panes. The Mac's
 * `FleetStore.Reading`, and the same four answers.
 *
 * Only an answering runner's count is worth adding up. Reporting a reconnecting
 * runner's last count as live is hands on deck that went home; reporting it as
 * nothing running is a death nobody saw. So a fleet with no runner answering
 * says neither.
 */
sealed interface FleetReading {
    /** At least one answering runner can read tmux: this many, across the answering runners only. */
    data class Live(val count: Int) : FleetReading

    /** Runners are answering, and not one of them can reach tmux. */
    data object RuntimeDown : FleetReading

    /** Nothing is known yet: every runner is still on its first read. */
    data object Connecting : FleetReading

    /** No runner is answering, and at least one has been lost or refused. */
    data object Unsaid : FleetReading
}

/** One runner, as [fleetReading] needs it: its link, and the count it last reported. */
data class RunnerCount(
    val link: RunnerLink,
    val count: Int,
    val runtimeHealthy: Boolean = true,
    /**
     * Whether its link is up, whatever its fleet can vouch for. Away and
     * connected is a link whose first fleet read failed ([FleetRead.FAILED]):
     * not "Not connected", which would be untrue, but can't say yet.
     */
    val connected: Boolean = false,
)

/**
 * The reading, from each runner's link and last count.
 *
 * Gated on [RunnerLink.ANSWERING], not on "not failed": the reconnect between
 * attempts is where a dead runner spends most of its outage. Healthy is an OR
 * across the answering runners, and the count is every answering runner's,
 * healthy or not, as the Mac counts it.
 */
fun fleetReading(runners: List<RunnerCount>): FleetReading {
    val answering = runners.filter { it.link == RunnerLink.ANSWERING }
    if (answering.isNotEmpty()) {
        if (answering.none { it.runtimeHealthy }) return FleetReading.RuntimeDown
        return FleetReading.Live(answering.sumOf { it.count })
    }
    return if (runners.all { it.link == RunnerLink.CONNECTING }) {
        FleetReading.Connecting
    } else {
        FleetReading.Unsaid
    }
}

/**
 * The drawer's footer: "3 live · 2 runners", or why it can't say.
 *
 * The count is the answering runners' only, so the footer says how many of
 * the runners it is from when that is not all of them: "3 live · 1 of 2
 * runners". "2 runners" alone read as three across both.
 *
 * "No runners" first, and not colored by anyone: an app nobody has added a
 * runner to yet is empty, not broken.
 */
fun liveSummary(runners: List<RunnerCount>): String {
    if (runners.isEmpty()) return "No runners"
    return when (val reading = fleetReading(runners)) {
        is FleetReading.Live -> {
            val answering = runners.count { it.link == RunnerLink.ANSWERING }
            val all = if (runners.size == 1) "1 runner" else "${runners.size} runners"
            val from = if (answering == runners.size) all else "$answering of $all"
            "${reading.count} live · $from"
        }
        FleetReading.RuntimeDown -> "tmux unavailable"
        FleetReading.Connecting -> "Connecting…"
        // A link that is up but whose fleet can't be read is connected, and
        // "Not connected" would be untrue of it: it can't say yet.
        FleetReading.Unsaid -> if (runners.any { it.connected }) "Can’t say yet" else "Not connected"
    }
}

/**
 * What is happening, given that nothing needs you: the front door's line under
 * "Nothing needs you".
 *
 * [runners] carries each runner's count of WORKING agents in the worktrees the
 * screen shows. [where] is " on <runner>" when there is exactly one runner, and
 * empty otherwise. A fleet with no runner answering says it can't say, rather
 * than "Nothing is running", which is the one sentence here that would be a
 * claim about a runner nobody has heard from.
 */
fun reassurance(
    runners: List<RunnerCount>,
    where: String,
    worktrees: Int,
    /** Workspaces exist and none has an orchestrator (ov-205): the one thing to do about an idle fleet. */
    noOrchestrator: Boolean = false,
): String {
    // No runners is not "connecting": there is nothing to connect to.
    if (runners.isEmpty()) return "Nothing is running."
    return when (val reading = fleetReading(runners)) {
        FleetReading.Connecting -> "Connecting…"
        FleetReading.Unsaid -> "Can’t say what’s running until a runner answers."
        // Not "Nothing is running": with tmux down, no pane could be read.
        FleetReading.RuntimeDown -> "Can’t say what’s running$where: tmux isn’t answering."
        is FleetReading.Live -> {
            val working = reading.count
            if (working == 0 && noOrchestrator) {
                // Purpose first: no agents are working, and the way to change that.
                FirstRunCopy.NO_ORCHESTRATOR_RUNNING
            } else if (working == 0 && worktrees == 0) {
                "Nothing is running$where yet."
            } else {
                when (working) {
                    0 -> "Nothing is running$where."
                    1 -> "One agent is working$where."
                    else -> "$working agents are working$where."
                }
            }
        }
    }
}
