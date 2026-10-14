package com.farcooler.model

// The Plan view as sections (ov-503, design 2): what moves main first, then what is queued, then what it costs, then
// why. Every rule is here, as values; the views only lay it out. The Android twin of AgentKit's `PlanSections.swift`,
// held to it by `test/fixtures/plan-sections-cases.json`.
//
// Order (owner decision 7; ov-522): Needs You, Landing (the trains), In Review, Building, Waiting, Agents, Themes. The
// word is landing, never shipping.
//
// A lane belongs to exactly one place:
// - on a live train, it's in that train's card (Landing), whatever its state;
// - naming a train that landed while it's still queued or landing, it's in On a landed train (folded, each row says
//   which train);
// - a lane building, in review or fixing is never hidden by a landed train: it routes by its state and the row says
//   it names that train ([LaneRow.landedTrain]);
// - in review, fixing, or landing with no train, it's In Review;
// - building, it's Building;
// - queued, it's Waiting on You when a card needs a decision, else Next Up when the plan ranks it, else Not Ranked.
// A lane named like a live train is that train's agent and is left out (ov-461).

data class PlanSections(
    /** The Needs You count the title bar says; the section's rows are the runner's own list, which the plan doesn't carry. */
    val needsYou: Int,
    /** Live trains, furthest along first. */
    val trains: List<TrainCard>,
    val inReview: List<LaneRow>,
    val building: List<LaneRow>,
    val waiting: Waiting,
    val fleet: PlanFleet,
    val themes: PlanThemeGrid,
    /** Queued or landing lanes that name a train that has landed. */
    val onLandedTrain: List<LaneRow>,
    /** Whether the runner lists agents (`board_agents`); without it the Agents section isn't drawn. */
    val agentsSupported: Boolean,
    /** The runner's clock. */
    val now: Long,
) {
    enum class Kind(val title: String, val emptyWords: String?) {
        NEEDS_YOU("Needs you", null),
        LANDING("Landing", "No train is landing."),
        IN_REVIEW("In review", null),
        BUILDING("Building", "Nothing is building."),
        WAITING("Waiting", "Nothing is queued."),
        ON_LANDED_TRAIN("On a landed train", null),
        AGENTS("Agents", PlanWords.NO_AGENTS),
        THEMES("Themes", null);

        /** Drawn folded until opened. */
        val startsFolded: Boolean get() = this == ON_LANDED_TRAIN
    }

    /** A lane row in In review, Building or Waiting. */
    data class LaneRow(
        val lane: PlanLane,
        val agent: PlanFleetAgent?,
        val waitsOnOwner: Boolean,
        /** The heading of the landed train the lane still names, or null. */
        val landedTrain: String? = null,
    ) {
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

    data class TrainLane(val lane: PlanLane, val standing: Standing, val waitsOnOwner: Boolean) {
        val id: String get() = lane.id
        val name: String get() = lane.name

        /**
         * What a lane aboard says about itself, or null when it is simply aboard (ov-522). "Ready" said nothing: the
         * train is being tested, ready for what? Only what matters is said: a lane that waits on the owner, and a lane
         * that has landed. (A lane never keeps its train from leaving, ov-523, so none says so.)
         */
        val note: String?
            get() = if (waitsOnOwner) "Needs you" else (standing as? Standing.Landed)?.word

        /** The note is drawn in amber: the lane waits on the owner. */
        val noteNeedsAttention: Boolean get() = waitsOnOwner
    }

    /** A live train and its lanes. */
    data class TrainCard(
        val train: PlanTrain,
        val phase: TrainPhase,
        val since: Long,
        val lanes: List<TrainLane>,
    ) {
        val id: String get() = train.id
    }

    data class Waiting(val next: List<LaneRow>, val notRanked: List<LaneRow>, val onYou: List<LaneRow>) {
        val isEmpty: Boolean get() = next.isEmpty() && notRanked.isEmpty() && onYou.isEmpty()
        val count: Int get() = next.size + notRanked.size + onYou.size
    }

    /**
     * The sections to draw, in order. Needs you, In review and Themes are drawn only with rows; Landing,
     * Waiting and Agents always are, and say so when empty; Building also when something is queued, since the gap
     * between Next and Landing is then worth saying. Agents is left out for a runner that lists none.
     */
    val shown: List<Kind>
        get() = Kind.entries.filter { kind ->
            when (kind) {
                Kind.NEEDS_YOU -> needsYou > 0
                Kind.LANDING, Kind.WAITING -> true
                Kind.ON_LANDED_TRAIN -> onLandedTrain.isNotEmpty()
                Kind.IN_REVIEW -> inReview.isNotEmpty()
                Kind.BUILDING -> building.isNotEmpty() || waiting.next.isNotEmpty()
                Kind.AGENTS -> agentsSupported
                Kind.THEMES -> !themes.isEmpty
            }
        }

    /** Rows in a section, for deciding whether it's empty. */
    fun count(kind: Kind): Int = when (kind) {
        Kind.NEEDS_YOU -> needsYou
        Kind.LANDING -> trains.size
        Kind.IN_REVIEW -> inReview.size
        Kind.BUILDING -> building.size
        Kind.WAITING -> waiting.count
        Kind.ON_LANDED_TRAIN -> onLandedTrain.size
        Kind.AGENTS -> fleet.agents.size
        Kind.THEMES -> themes.tiles.size + themes.finished.size
    }

    /** What the section says now: its empty words with nothing in it, else null. */
    fun emptyWords(kind: Kind): String? =
        if (count(kind) != 0) null else if (kind == Kind.AGENTS) fleet.header() else kind.emptyWords

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

            // A live lane on a landed train names it still.
            val landedTrains = plan.trains.filter { it.state == TrainState.LANDED }
            fun landed(lane: PlanLane): PlanTrain? = landedTrains.firstOrNull { train ->
                train.lanes.any { it.lane == lane.id } || lane.train.equals(train.name, ignoreCase = true)
            }

            fun row(lane: PlanLane) = LaneRow(lane, agentOf(lane), plan.waitsOnOwner(lane, statuses), landed(lane)?.label)

            // Trains, furthest along first; the plan's order breaks a tie.
            val live = plan.trains.filter { !it.state.isSettled }
                .withIndex().sortedWith(compareByDescending<IndexedValue<PlanTrain>> { it.value.phaseIn(plan).progress }.thenBy { it.index })
                .map { it.value }
            val onLive = mutableSetOf<String>()
            val trains = live.map { train ->
                val phase = train.phaseIn(plan)
                val since = train.phaseSinceIn()
                val mine = plan.lanesOn(train).filter { it.id !in isTrainAgent }
                onLive += mine.map { it.id }
                TrainCard(
                    train, phase, since,
                    mine.map { TrainLane(it, standing(it), plan.waitsOnOwner(it, statuses)) },
                )
            }

            val free = candidates.filter { it.state.isLive && it.id !in onLive }
            // Only queued and landing lanes go to the folded section; the rest route by their state and are flagged.
            val parked = free.filter { (it.state == LaneState.QUEUED || it.state == LaneState.LANDING) && landed(it) != null }
            val parkedIds = parked.map { it.id }.toSet()
            val loose = free.filter { it.id !in parkedIds }

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
            val moving = (trains.flatMap { c -> c.lanes.map { it.id } } + (inReview + building).map { it.id }).toSet()
            return PlanSections(
                needsYou = maxOf(0, needsYou), trains = trains, inReview = inReview.map(::row), building = building.map(::row),
                waiting = waiting, fleet = PlanFleet.of(plan.agents, plan.agentsSpend),
                themes = PlanThemeGrid.of(plan, moving, statuses, activity),
                onLandedTrain = parked.map(::row), agentsSupported = agentsSupported, now = plan.nowMs,
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
