package com.farcooler.model

/**
 * What needs a person, across every runner this phone is connected to.
 *
 * The derivation behind the front door, kept out of the composable so it can be
 * tested without a device — the same split `ui/Navigation.kt` makes for the back
 * stack. Everything here is pure: a list of workspaces in, an ordered list of
 * sections out.
 *
 * ## The unit is a workspace, and everything in it is a target
 *
 * One section per workspace: what the work is in the header, a row for each
 * agent inside it that wants a person — blocked first, then finished — and a
 * row for its diff. A workspace that is blocked and finished and unread appears
 * once, saying all three, and every part of what it says is something you can
 * tap. iOS arrived at this in `43a320f` after two row KINDS produced four rows
 * for one piece of work.
 *
 * ## What is not a port: this spans every runner
 *
 * iOS's inbox is scoped to one `Connection` and its screen carries a
 * `HostSwitcherBar` so you can go and look at the others. This app connects to
 * every runner at once — `net/FleetRepository.kt` — because the product's claim
 * is that an agent blocked on a runner in another room is exactly as urgent as
 * one on this desk, and a picker answers that claim by asking you to go and
 * check. So a front door grouped by RUNNER would be that picker again in list
 * form: the most urgent thing in the fleet could sit halfway down the screen,
 * under a heading for a machine with nothing to say.
 *
 * The grouping is therefore by workspace and the ordering spans runners, with
 * [NeedsYouSection.hostId] carried on every section so a tap acts on the right
 * daemon. Ids are minted per daemon, so [NeedsYouSection.key] is `host/workspace`
 * and never the workspace alone.
 *
 * ## Ranks from two runners are comparable, and that is not obvious
 *
 * `farcooler_core::feed::rank` is `tier * TIER_SPAN + (TIER_SPAN - 1 - age)`.
 * The tier is a shared enum — blocked, then done-or-failed, then working, then
 * idle — and the age is a DURATION in seconds, measured by each daemon against
 * its own state change. Neither term is a wall-clock instant, so nothing here
 * compares one machine's clock with another's, and two blocked agents on two
 * runners sort by which has been stuck longer.
 *
 * That is worth writing down because the rest of this model does not have the
 * property: [Terminal.activitySince] IS a wall-clock instant taken on the
 * runner, and `ModelTest` pins what happens when a runner's clock runs ahead of
 * the phone's. Rank sidesteps the whole question, which is what makes a merged
 * front door possible at all.
 *
 * A daemon too old to send `rank` gets [Terminal.sortRank]'s `Long.MAX_VALUE`
 * and sorts last within its tier — an unknown must not outrank a known blocked
 * agent.
 */

/** One workspace, its runner, and what that runner said about its diff. */
data class NeedsYouInput(
    val hostId: String,
    /** The runner's own name, shown on a section only when more than one is connected. */
    val hostLabel: String,
    val workspace: Workspace,
    /**
     * This worktree's `changes.inbox` row, or null.
     *
     * Null is the absence of a fact, not the fact that there is nothing. A
     * runner that has not answered yet — and one whose daemon predates
     * `changes.inbox` and never will — is null; a worktree the runner has
     * called clean is a row with no diff. The two get opposite answers from
     * [NeedsYouSection.showsChanges].
     */
    val counts: InboxRow? = null,
)

/**
 * One workspace and every reason it is on the front door.
 *
 * A single type rather than the two cases iOS replaced, because two cases were
 * two rows for one piece of work. The tiers are an ordering over these, not a
 * difference in kind.
 */
data class NeedsYouSection(
    val hostId: String,
    val hostLabel: String,
    val workspace: Workspace,
    /** Its blocked agents, most urgent first. Empty on a section here for something else. */
    val blocked: List<Terminal>,
    /**
     * Its finished agents — `done`, which is finished and UNSEEN — most urgent
     * first.
     *
     * A separate list rather than more entries in [blocked], because the two
     * are counted and worded differently when a section runs out of room: "2
     * more agents need you" and "2 more agents finished" are not the same
     * sentence, and one list of five would have to say one of them about both.
     */
    val finished: List<Terminal>,
    val counts: InboxRow?,
    /** [Workspace.ordinals], computed once per section rather than once per row. */
    val ordinals: Map<String, Int>,
) {
    /**
     * Identity, and the runner is half of it. Two runners can hold workspaces
     * with the same id — `BackstackTest` pins that they must not be conflated —
     * and this string is what a `LazyColumn` keys its items on.
     */
    val key: String get() = "$hostId/${workspace.id}"

    /**
     * Whether this section draws a row for its diff.
     *
     * Absent on a worktree the runner has called clean: `+0 -0` on every
     * section is noise in the shape of information, and a row onto an empty
     * diff is the same noise with more height.
     *
     * PRESENT while the runner has not answered yet, which is a different state
     * and gets the opposite answer — see [NeedsYouInput.counts]. A row that
     * appeared one poll later would push every row under it down, under a thumb
     * already travelling toward one of them.
     */
    val showsChanges: Boolean get() = counts?.hasDiff != false
}

/**
 * How many agents of ONE KIND a section shows before it starts counting them.
 *
 * Three, because each one is a fleet row up to eight lines tall and a workspace
 * with eight blocked agents would otherwise be the whole screen. What is lost
 * is small: the tab strip on the other side of the tap holds every one of them,
 * labelled.
 *
 * Spent per kind rather than over the agents together. A shared budget of
 * three, spent blocked-first, would hide an answer behind three open questions
 * in the same worktree — and an answer arriving is the thing this screen most
 * needs to say, not an edge case it tolerates.
 */
const val AGENTS_PER_WORKSPACE = 3

/**
 * Every workspace that wants a person, in the order it wants them.
 *
 * Two tiers, and the first one holds two kinds.
 *
 * 1. **Workspaces with an agent wanting attention**, ranked by the LOWEST
 *    [Terminal.sortRank] among them. A workspace is as urgent as its most
 *    urgent agent; an average or a count would let a worktree with six working
 *    agents outrank one with a single agent stuck for an hour.
 * 2. **Unread diffs** — `changedSinceReviewed && hasDiff` with no agent wanting
 *    anything — in the order the fleet arrived in, which is runner order and
 *    then each runner's own.
 *
 * **Blocked before finished, and this function does not arrange it.**
 * `feed::rank` already puts every blocked agent a whole `TIER_SPAN` below every
 * finished one, so filtering on the shared [AgentActivity.wantsAttention] and
 * taking the lowest rank yields blocked-above-finished for free, both across
 * sections and inside one. Nothing here re-scores: a second opinion about which
 * agent matters is the exact thing that number exists to prevent.
 *
 * **A finished agent goes above an unread diff**, which is what the two tiers
 * buy. A diff sits still — it was true before the app was opened and stays true
 * until it is read. A finished turn is news with somebody waiting on the other
 * end, and it expires the moment it is read. The perishable thing goes above
 * the durable one.
 *
 * The second tier has no rank of its own, deliberately. [InboxRow] is a
 * workspace's counts, not an agent's state, and inventing a rank from the
 * workspace's terminals would sort a diff by how blocked some agent in the same
 * worktree happens to be, which is not a fact about the diff.
 *
 * **The tiebreak is the runner and then the workspace**, and it is load-bearing
 * twice over. Ranks genuinely collide — two agents that entered the same tier
 * in the same second get the same number — and across runners they collide more
 * often, because two daemons that each have one agent blocked for four minutes
 * produce the identical integer. Without a total order two equally-ranked
 * sections could swap places on any poll, and a row that moves under a finger
 * already travelling toward it is a tap that lands on something else.
 *
 * **Hidden workspaces are not here.** A hidden workspace is one the user asked
 * not to see; a front door that shows what you hid is not honoring the hiding.
 * They are one tap away in the workspace list, behind the same disclosure they
 * have always been.
 */
fun needsYou(inputs: List<NeedsYouInput>): List<NeedsYouSection> {
    val attention = mutableListOf<Pair<Long, NeedsYouSection>>()
    val unread = mutableListOf<NeedsYouSection>()

    for (input in inputs) {
        val workspace = input.workspace
        if (workspace.isHidden) continue

        // `wantsAttention` and not a list of cases, because that property IS
        // the product's single definition of what is worth interrupting
        // somebody for, shared with the Mac and derived on the host. A screen
        // writing its own copy of the answer is how iOS came to have one
        // surface that disagreed with it.
        //
        // Sorted once and then split. The split does not reorder: rank puts
        // every blocked agent a whole tier below every finished one, so the two
        // slices come out already in the order they are drawn in.
        val wanting = workspace.terminals
            .filter { it.agent.wantsAttention }
            .sortedWith(compareBy({ it.sortRank }, { it.id }))

        val section = NeedsYouSection(
            hostId = input.hostId,
            hostLabel = input.hostLabel,
            workspace = workspace,
            blocked = wanting.filter { it.agent == AgentActivity.BLOCKED },
            finished = wanting.filter { it.agent == AgentActivity.DONE },
            counts = input.counts,
            ordinals = workspace.ordinals(),
        )

        val first = wanting.firstOrNull()
        if (first != null) {
            // A workspace whose only news is a finished agent ranks by that
            // agent, which lands it below every blocked workspace and above
            // every unread diff without this line knowing which kind it holds.
            attention += first.sortRank to section
        } else if (input.counts?.changedSinceReviewed == true && input.counts.hasDiff) {
            unread += section
        }
    }

    attention.sortWith(
        compareBy({ it.first }, { it.second.hostId }, { it.second.workspace.id })
    )
    return attention.map { it.second } + unread
}

/**
 * The blocked agents a section ran out of room for.
 *
 * A sentence rather than a bare number, because "+2" under a list of agents
 * reads as two more of something and does not say what.
 */
fun blockedOverflow(count: Int): String =
    if (count == 1) "1 more agent needs you" else "$count more agents need you"

/**
 * The finished agents a section ran out of room for.
 *
 * Says "failed" when any of the hidden ones did, and the words are the daemon's
 * rather than a third set: `watch.rs` titles a finished turn "<name> finished"
 * and a died-halfway one "<name> failed", and [Terminal.activityLabel] puts the
 * same distinction on the rows above this line. A sentence that swept a failure
 * into "finished" would hide the one thing in the group most worth going in for
 * — in the only place on this screen where agents are counted instead of shown.
 *
 * Takes the terminals rather than a count, because a count cannot answer which
 * of them failed.
 */
fun finishedOverflow(hidden: List<Terminal>): String {
    val failures = hidden.count { it.turnDidFail }
    if (failures == hidden.size) {
        return if (failures == 1) "1 more agent failed" else "$failures more agents failed"
    }
    if (failures > 0) return "${hidden.size} more agents finished, $failures failed"
    return if (hidden.size == 1) "1 more agent finished" else "${hidden.size} more agents finished"
}
