package com.farcooler.model

// The Plan view as sections (ov-503, design 2): what moves main first, then what is queued, then what it costs, then
// why. Every rule is here, as values; the views only lay it out. The Android twin of AgentKit's `PlanSections.swift`,
// held to it by `test/fixtures/plan-sections-cases.json`.
//
// Order (owner decision 7): Needs You, Shipping, In Review, Building, Waiting, Agents, Themes.
//
// A lane belongs to exactly one place:
// - on a live train, it's in that train's card (Shipping), whatever its state;
// - on a train that landed but still reads live, it's `stranded`, in no section: its commits are on main;
// - in review, fixing, or landing with no train, it's In Review;
// - building, it's Building;
// - queued, it's Waiting on You when a card needs a decision, else Next Up when the plan ranks it, else Not Ranked.
// A lane named like a live train is that train's agent and is left out (ov-461).

data class PlanSections(
    /** The Needs You count the title bar says; the section's rows are the runner's own list, which the plan doesn't carry. */
    val needsYou: Int,
    /** Live trains, furthest along first. */
    val shipping: List<TrainCard>,
    val inReview: List<LaneRow>,
    val building: List<LaneRow>,
    val waiting: Waiting,
    val fleet: PlanFleet,
    val themes: PlanThemeGrid,
    /** Live lanes on a train that has landed: on main already, in no section. */
    val stranded: List<PlanLane>,
    /** Whether the runner lists agents (`board_agents`); without it the Agents section isn't drawn. */
    val agentsSupported: Boolean,
    /** The runner's clock. */
    val now: Long,
) {
    enum class Kind(val title: String, val emptyWords: String?) {
        NEEDS_YOU("Needs you", null),
        SHIPPING("Shipping", "No train is shipping."),
        IN_REVIEW("In review", null),
        BUILDING("Building", "Nothing is building."),
        WAITING("Waiting", "Nothing is queued."),
        AGENTS("Agents", PlanWords.NO_AGENTS),
        THEMES("Themes", null),
    }

    /** A lane row in In review, Building or Waiting. */
    data class LaneRow(val lane: PlanLane, val agent: PlanFleetAgent?, val waitsOnOwner: Boolean) {
        val id: String get() = lane.id
    }

    /** How a lane stands relative to the train it's on. */
    sealed interface Standing {
        val word: String

        data object Ready : Standing { override val word = "Ready" }
        data class Fixing(val round: Int) : Standing {
            override val word get() = if (round > 0) "Fixing · round $round" else "Fixing"
        }
        data class Landed(val sha: String?) : Standing {
            override val word get() = if (sha.isNullOrEmpty()) "Landed" else "Landed · ${sha.take(8)}"
        }
        /** Not ready yet: queued, building or in review. */
        data class NotReady(val state: LaneState) : Standing {
            override val word get() = PlanWords.state(state)
        }
    }

    data class TrainLane(val lane: PlanLane, val standing: Standing, val holding: Boolean, val waitsOnOwner: Boolean) {
        val id: String get() = lane.id
        val name: String get() = lane.name
    }

    /** A live train and its lanes. */
    data class TrainCard(
        val train: PlanTrain,
        val phase: TrainPhase,
        val since: Long,
        val lanes: List<TrainLane>,
        /** "Holding for ios-overwrite · 2 h", or null. */
        val held: String?,
    ) {
        val id: String get() = train.id
    }

    data class Waiting(val next: List<LaneRow>, val notRanked: List<LaneRow>, val onYou: List<LaneRow>) {
        val isEmpty: Boolean get() = next.isEmpty() && notRanked.isEmpty() && onYou.isEmpty()
        val count: Int get() = next.size + notRanked.size + onYou.size
    }

    /**
     * The sections to draw, in order. Needs you, In review and Themes are drawn only with rows; Shipping, Waiting and
     * Agents always are, and say so when empty; Building also when something is queued, since the gap between Next and
     * Shipping is then worth saying. Agents is left out for a runner that lists none.
     */
    val shown: List<Kind>
        get() = Kind.entries.filter { kind ->
            when (kind) {
                Kind.NEEDS_YOU -> needsYou > 0
                Kind.SHIPPING, Kind.WAITING -> true
                Kind.IN_REVIEW -> inReview.isNotEmpty()
                Kind.BUILDING -> building.isNotEmpty() || waiting.next.isNotEmpty()
                Kind.AGENTS -> agentsSupported
                Kind.THEMES -> !themes.isEmpty
            }
        }

    /** Rows in a section, for deciding whether it's empty. */
    fun count(kind: Kind): Int = when (kind) {
        Kind.NEEDS_YOU -> needsYou
        Kind.SHIPPING -> shipping.size
        Kind.IN_REVIEW -> inReview.size
        Kind.BUILDING -> building.size
        Kind.WAITING -> waiting.count
        Kind.AGENTS -> fleet.working.size + fleet.finished.size
        Kind.THEMES -> themes.tiles.size + themes.finished.size
    }

    /** What the section says now: its empty words with nothing in it, else null. */
    fun emptyWords(kind: Kind): String? = if (count(kind) == 0) kind.emptyWords else null

    companion object {
        /**
         * @param needsYou the workspace's Needs You count.
         * @param statuses the board's card statuses by task id, which say whether a card waits on a decision and which
         *   glyph a theme's card gets.
         * @param activity when each card last moved (ms), by task id.
         * @param agentsSupported whether the runner advertises `board_agents`.
         */
        fun of(
            plan: Plan,
            needsYou: Int = 0,
            statuses: Map<String, TaskStatus> = emptyMap(),
            activity: Map<String, Long> = emptyMap(),
            agentsSupported: Boolean = true,
        ): PlanSections {
            val isTrainAgent = plan.laneIsTrain.map { it.lane }.toSet()
            val candidates = plan.lanes.filter { it.id !in isTrainAgent }

            fun agentOf(lane: PlanLane): PlanFleetAgent? = plan.agents
                .filter { it.laneId == lane.id && it.role != PlanFleetRole.ORCHESTRATOR }
                .sortedWith(compareByDescending<PlanFleetAgent> { it.working }.thenByDescending { it.spend.totalTokens })
                .firstOrNull()

            fun row(lane: PlanLane) = LaneRow(lane, agentOf(lane), plan.waitsOnOwner(lane, statuses))

            // Trains, furthest along first; the plan's order breaks a tie.
            val live = plan.trains.filter { !it.state.isSettled }
                .withIndex().sortedWith(compareByDescending<IndexedValue<PlanTrain>> { it.value.phaseIn(plan).progress }.thenBy { it.index })
                .map { it.value }
            val onLive = mutableSetOf<String>()
            val shipping = live.map { train ->
                val phase = train.phaseIn(plan)
                val since = train.phaseSinceIn()
                val holders = train.heldBy.map { it.lane }.toSet()
                val mine = plan.lanesOn(train).filter { it.id !in isTrainAgent }
                onLive += mine.map { it.id }
                TrainCard(
                    train, phase, since,
                    mine.map { TrainLane(it, standing(it), it.id in holders, plan.waitsOnOwner(it, statuses)) },
                    PlanWords.held(train, since, plan.nowMs),
                )
            }

            // A live lane on a landed train is on main already.
            val landedTrains = plan.trains.filter { it.state == TrainState.LANDED }
            fun onLanded(lane: PlanLane) = landedTrains.any { train ->
                train.lanes.any { it.lane == lane.id } || lane.train.equals(train.name, ignoreCase = true)
            }
            val free = candidates.filter { it.state.isLive && it.id !in onLive }
            val stranded = free.filter(::onLanded)
            val loose = free.filter { !onLanded(it) }

            val queued = loose.filter { it.state == LaneState.QUEUED }
            val blocked = queued.filter { plan.waitsOnOwner(it, statuses) }
            val blockedIds = blocked.map { it.id }.toSet()
            val ranked = plan.order.mapNotNull { id -> queued.firstOrNull { it.id == id && id !in blockedIds } }
            val rankedIds = ranked.map { it.id }.toSet()
            val waiting = Waiting(
                next = ranked.map(::row),
                notRanked = queued.filter { it.id !in rankedIds && it.id !in blockedIds }.map(::row),
                onYou = blocked.map(::row),
            )
            val inReview = loose.filter { it.state == LaneState.REVIEW || it.state == LaneState.FIXING || it.state == LaneState.LANDING }
            val building = loose.filter { it.state == LaneState.BUILDING }
            val moving = (shipping.flatMap { c -> c.lanes.map { it.id } } + (inReview + building).map { it.id }).toSet()
            return PlanSections(
                needsYou = maxOf(0, needsYou), shipping = shipping, inReview = inReview.map(::row), building = building.map(::row),
                waiting = waiting, fleet = PlanFleet(plan.agents),
                themes = PlanThemeGrid.of(plan, moving, statuses, activity),
                stranded = stranded, agentsSupported = agentsSupported, now = plan.nowMs,
            )
        }

        /** A lane's standing on its train. */
        internal fun standing(lane: PlanLane): Standing = when (lane.state) {
            LaneState.LANDING -> Standing.Ready
            LaneState.FIXING -> Standing.Fixing(lane.fixRounds)
            LaneState.LANDED -> Standing.Landed(lane.landedSha)
            else -> Standing.NotReady(lane.state)
        }
    }
}
