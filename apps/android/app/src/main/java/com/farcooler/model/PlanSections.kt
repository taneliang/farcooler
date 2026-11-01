package com.farcooler.model

// The Plan view as sections (ov-503, design 2; ov-872): what moves main first, then what is waiting, then what it costs,
// then why. Every rule is here, as values; the views only lay it out. The Android twin of AgentKit's `PlanSections.swift`,
// held to it by `test/fixtures/plan-sections-cases.json`.
//
// Order (owner decision 7; ov-522): Needs You, Landing (the landings), In Review, Building, Waiting, Agents, Themes. The
// word is landing, never shipping.
//
// A ticket belongs to exactly one place:
// - on a live landing, it's in that landing's card (Landing), whatever its stage;
// - reviewing, being fixed, at Needs you, or landing with no live landing, it's In Review;
// - building, it's Building;
// - waiting, it's Waiting on You when its card needs a decision, else Next Up when the plan ranks it, else Not Ranked.
// A ticket that has landed or been dropped is in no section.

data class PlanSections(
    /** The Needs You count the title bar says; the section's rows are the runner's own list, which the plan doesn't carry. */
    val needsYou: Int,
    /** Live landings, furthest along first. */
    val landings: List<LandingCard>,
    val inReview: List<TicketRow>,
    val building: List<TicketRow>,
    val waiting: Waiting,
    val fleet: PlanFleet,
    val themes: PlanThemeGrid,
    /** The runner's clock. */
    val now: Long,
) {
    enum class Kind(val title: String, val emptyWords: String?) {
        NEEDS_YOU("Needs you", null),
        LANDING("Landing", "Nothing is landing right now."),
        IN_REVIEW("In review", null),
        BUILDING("Building", "Nothing is building."),
        WAITING("Waiting", "Nothing is waiting."),
        AGENTS("Agents", PlanWords.NO_AGENTS),
        THEMES("Themes", null);
    }

    /** A ticket row in In review, Building or Waiting. */
    data class TicketRow(
        val ticket: PlanTicket,
        /** The agent working it now, working ones first. */
        val agent: PlanAgent?,
        val waitsOnOwner: Boolean,
    ) {
        val id: String get() = ticket.task
    }

    /** How a ticket stands relative to the landing it's on. */
    sealed interface Standing {
        val word: String

        data object Ready : Standing { override val word = "Ready" }
        data class Fixing(val round: Int) : Standing {
            override val word get() = if (round > 0) "Fixing · round $round" else "Fixing"
        }
        data class Landed(val sha: String?) : Standing {
            override val word get() = if (sha.isNullOrEmpty()) "Landed" else "Landed · ${sha.take(8)}"
        }
        /** Not ready yet: waiting, building or in review. */
        data class NotReady(val stage: TicketStage) : Standing {
            override val word get() = PlanWords.state(stage)
        }
    }

    data class LandingTicket(val ticket: PlanTicket, val standing: Standing, val waitsOnOwner: Boolean) {
        val id: String get() = ticket.task
        val key: String get() = ticket.key

        /**
         * What a ticket aboard says about itself, or null when it is simply aboard (ov-522). "Ready" said nothing: the
         * landing is being tested, ready for what? Only what matters is said: a ticket that waits on the owner, and a
         * ticket that has landed.
         */
        val note: String?
            get() = if (waitsOnOwner) "Needs you" else (standing as? Standing.Landed)?.word

        /** The note is drawn in amber: the ticket waits on the owner. */
        val noteNeedsAttention: Boolean get() = waitsOnOwner
    }

    /** A live landing and its tickets. */
    data class LandingCard(
        val landing: PlanLanding,
        val phase: LandingPhase,
        val since: Long,
        val tickets: List<LandingTicket>,
        /** The agent landing it, when the plan lists one. */
        val lander: PlanAgent? = null,
    ) {
        val id: String get() = landing.id
    }

    data class Waiting(val next: List<TicketRow>, val notRanked: List<TicketRow>, val onYou: List<TicketRow>) {
        val isEmpty: Boolean get() = next.isEmpty() && notRanked.isEmpty() && onYou.isEmpty()
        val count: Int get() = next.size + notRanked.size + onYou.size
    }

    /**
     * The sections to draw, in order. Needs you, In review and Themes are drawn only with rows; Landing,
     * Waiting and Agents always are, and say so when empty; Building also when something is waiting, since the gap
     * between Next and Landing is then worth saying.
     */
    val shown: List<Kind>
        get() = Kind.entries.filter { kind ->
            when (kind) {
                Kind.NEEDS_YOU -> needsYou > 0
                Kind.LANDING, Kind.WAITING, Kind.AGENTS -> true
                Kind.IN_REVIEW -> inReview.isNotEmpty()
                Kind.BUILDING -> building.isNotEmpty() || waiting.next.isNotEmpty()
                Kind.THEMES -> !themes.isEmpty
            }
        }

    /** Rows in a section, for deciding whether it's empty. */
    fun count(kind: Kind): Int = when (kind) {
        Kind.NEEDS_YOU -> needsYou
        Kind.LANDING -> landings.size
        Kind.IN_REVIEW -> inReview.size
        Kind.BUILDING -> building.size
        Kind.WAITING -> waiting.count
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
         */
        fun of(
            plan: Plan,
            needsYou: Int = 0,
            statuses: Map<String, TaskStatus> = emptyMap(),
            activity: Map<String, Long> = emptyMap(),
        ): PlanSections {
            fun row(ticket: PlanTicket) = TicketRow(ticket, plan.agentsOn(ticket).firstOrNull(), plan.waitsOnOwner(ticket, statuses))

            // Landings, furthest along first; the plan's order breaks a tie.
            val live = plan.landings.filter { !it.state.isSettled }
                .withIndex().sortedWith(compareByDescending<IndexedValue<PlanLanding>> { it.value.phaseIn(plan).progress }.thenBy { it.index })
                .map { it.value }
            val onLive = mutableSetOf<String>()
            val landings = live.map { landing ->
                val phase = landing.phaseIn(plan)
                val since = landing.phaseSinceIn()
                val mine = plan.ticketsOn(landing)
                onLive += mine.map { it.task }
                LandingCard(
                    landing, phase, since,
                    mine.map { LandingTicket(it, standing(it), plan.waitsOnOwner(it, statuses)) },
                    lander = landing.lander?.let { id -> plan.agents.firstOrNull { it.id == id } },
                )
            }

            val free = plan.tickets.filter { it.stage.isLive && it.task !in onLive }
            val waitingTickets = free.filter { it.stage == TicketStage.WAITING }
            val blocked = waitingTickets.filter { plan.waitsOnOwner(it, statuses) }
            val blockedIds = blocked.map { it.task }.toSet()
            val ranked = waitingTickets.filter { it.planRank != null && it.task !in blockedIds }.sortedBy { it.planRank }
            val rankedIds = ranked.map { it.task }.toSet()
            val waiting = Waiting(
                next = ranked.map(::row),
                notRanked = waitingTickets.filter { it.task !in rankedIds && it.task !in blockedIds }.map(::row),
                onYou = blocked.map(::row),
            )
            val inReview = free.filter {
                it.stage == TicketStage.REVIEWING || it.stage == TicketStage.NEEDS_YOU || it.stage == TicketStage.LANDING || it.isFixing
            }
            val building = free.filter { it.stage == TicketStage.BUILDING && !it.isFixing }
            val moving = (landings.flatMap { c -> c.tickets.map { it.id } } + (inReview + building).map { it.task }).toSet()
            return PlanSections(
                needsYou = maxOf(0, needsYou), landings = landings, inReview = inReview.map(::row), building = building.map(::row),
                waiting = waiting, fleet = PlanFleet.of(plan.agents, plan.agentsSpend),
                themes = PlanThemeGrid.of(plan, moving, statuses, activity), now = plan.nowMs,
            )
        }

        /** A ticket's standing on its landing. */
        internal fun standing(ticket: PlanTicket): Standing = when {
            ticket.stage == TicketStage.LANDING -> Standing.Ready
            ticket.isFixing -> Standing.Fixing(ticket.fixRounds)
            ticket.stage == TicketStage.LANDED -> Standing.Landed(ticket.landedSha)
            else -> Standing.NotReady(ticket.stage)
        }
    }
}
