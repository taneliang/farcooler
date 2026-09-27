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
 * "No runners" first, and not colored by anyone: an app nobody has added a
 * runner to yet is empty, not broken.
 */
fun liveSummary(runners: List<RunnerCount>): String {
    if (runners.isEmpty()) return "No runners"
    return when (val reading = fleetReading(runners)) {
        is FleetReading.Live -> {
            val count = if (runners.size == 1) "1 runner" else "${runners.size} runners"
            "${reading.count} live · $count"
        }
        FleetReading.RuntimeDown -> "tmux unavailable"
        FleetReading.Connecting -> "Connecting…"
        FleetReading.Unsaid -> "Not connected"
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
fun reassurance(runners: List<RunnerCount>, where: String, worktrees: Int): String {
    // No runners is not "connecting": there is nothing to connect to.
    if (runners.isEmpty()) return "Nothing is running."
    return when (val reading = fleetReading(runners)) {
        FleetReading.Connecting -> "Connecting…"
        FleetReading.Unsaid -> "Can’t say what’s running until a runner answers."
        FleetReading.RuntimeDown, is FleetReading.Live -> {
            val working = (reading as? FleetReading.Live)?.count ?: 0
            if (working == 0 && worktrees == 0) {
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
