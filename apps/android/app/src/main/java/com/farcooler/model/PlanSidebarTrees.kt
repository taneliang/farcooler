package com.farcooler.model

import java.util.Locale

// The two-list sidebar (ov-503, design 3): the why and the how, apart. `ThemeTree` is Themes › Cards; `LaneTree` is
// Lanes › Agents, grouped as the Plan view's sections are, so the two read in one order. They replace `OneTree`'s
// single Theme › Task › Lane › Terminals outline. Terminals are the app's own (live panes, not plan data) and hang under
// a lane node in the view. The Android twin of AgentKit's `PlanSidebarTrees.swift`.
//
// A lane that works two cards still shows the join: [ThemeTree.laneOf] and [LaneTree.cardsOf] answer both ways, and a
// view highlights the other side when one is chosen.

data class ThemeTree(val filter: OneTree.Filter, val themes: List<ThemeNode>) {
    data class CardNode(
        val task: String,
        val key: String,
        val title: String,
        val status: TaskStatus,
        /** The lane working it now, shown as the trailing detail. */
        val lane: String?,
        val laneId: String?,
    ) {
        val glyph: PlanCardGlyph get() = PlanCardGlyph.of(status)
    }

    data class ThemeNode(val id: String, val name: String, val done: Int, val total: Int, val cards: List<CardNode>) {
        /** "6/10", the sidebar's short form. */
        val progress: String get() = "$done/$total"
    }

    /** The lane working a card, by the card's task id, or null. */
    fun laneOf(task: String): String? = themes.firstNotNullOfOrNull { t -> t.cards.firstOrNull { it.task == task } }?.laneId

    /** The cards a lane works, by its id, across themes. */
    fun cardsOf(lane: String): List<CardNode> = themes.flatMap { it.cards }.filter { it.laneId == lane }

    companion object {
        /**
         * Themes in the plan's order (active, paused, done), each with the cards [filter] shows. The filter applies to
         * cards, and a theme with none left isn't listed, except that a theme with no cards at all stays under Not done
         * and All, so a new one can be found.
         */
        fun of(
            plan: Plan,
            filter: OneTree.Filter = OneTree.Filter.OPEN,
            statuses: Map<String, TaskStatus> = emptyMap(),
        ): ThemeTree {
            // A train's agent isn't a lane, so it isn't "the lane working" a card. A lane naming a landed train still
            // is: live work never vanishes.
            val gone = plan.laneIsTrain.map { it.lane }.toSet()
            val themes = plan.shownThemes.mapNotNull { theme ->
                val cards = theme.cards.mapNotNull { ref ->
                    val known = plan.card(ref.task)
                    val status = statuses[ref.task] ?: known?.let { OneTree.status(it.status) } ?: return@mapNotNull null
                    val shown = when (filter) {
                        OneTree.Filter.OPEN -> status != TaskStatus.DONE && status != TaskStatus.CANCELLED
                        OneTree.Filter.IN_REVIEW -> status == TaskStatus.IN_REVIEW
                        OneTree.Filter.ALL -> true
                    }
                    if (!shown) return@mapNotNull null
                    val lane = liveLane(plan, ref.task, gone)
                    CardNode(ref.task, ref.key, known?.title ?: "", status, lane?.name, lane?.id)
                }
                if (cards.isEmpty() && !(theme.cards.isEmpty() && filter != OneTree.Filter.IN_REVIEW && theme.state == "active")) return@mapNotNull null
                ThemeNode(theme.id, theme.name, theme.counts.done, PlanWords.total(theme.counts), cards)
            }
            return ThemeTree(filter, themes)
        }

        /** The live lane working a card: one past queued first, else the queued one ranked highest. Null for a card whose lanes have all landed. */
        internal fun liveLane(plan: Plan, task: String, gone: Set<String>): PlanLane? {
            val mine = plan.lanes.filter { lane -> lane.state.isLive && lane.id !in gone && lane.cards.any { it.task == task } }
            return mine.firstOrNull { it.state != LaneState.QUEUED } ?: mine.minByOrNull { it.planRank ?: Int.MAX_VALUE }
        }
    }
}

data class LaneTree(
    /** Landing (its trains inside it) first, then In review, Building and the folded Waiting; a group with no lanes isn't listed. */
    val groups: List<Group>,
) {
    data class AgentNode(
        val id: String,
        /** "Builder · Opus". */
        val who: String,
        val working: Boolean,
        /** "1.2M"; null when no turn stated any. */
        val tokens: String?,
        /** The pane it ran in, when the runner read one: a row opens it. */
        val terminal: String? = null,
    )

    data class LaneNode(
        val id: String,
        val name: String,
        val heading: String,
        /**
         * The state beside it: on a train, only what matters ("Waits for you", "Landed · 3f78754f"), and nothing for a
         * lane that is simply aboard; off one, its state.
         */
        val state: String,
        val agents: List<AgentNode>,
        val cards: List<PlanCardRef>,
    )

    /** A train inside the Landing group: "Train 86", what it is for, where it stands, and its lanes (ov-522). */
    data class TrainNode(
        val id: String,
        /** "Train 86". */
        val label: String,
        /** Its title, under the label; null when it has none. */
        val subtitle: String?,
        /** "Checking on CI", the ladder step it stands on. */
        val phase: String,
        val lanes: List<LaneNode>,
    )

    sealed interface Kind {
        data object Landing : Kind
        data object InReview : Kind
        data object Building : Kind
        data object Waiting : Kind
        data object OnLandedTrain : Kind
    }

    data class Group(
        val kind: Kind,
        val title: String,
        /** A count for a folded group. */
        val detail: String?,
        /** Waiting starts folded. */
        val folded: Boolean,
        val lanes: List<LaneNode>,
        /** The trains of the Landing group, each with its lanes inside it; empty for every other group. */
        val trains: List<TrainNode> = emptyList(),
    )

    /** The cards a lane works, by the lane's id: what a view highlights when a lane is chosen. */
    fun cardsOf(lane: String): List<PlanCardRef> =
        groups.asSequence().flatMap { it.lanes + it.trains.flatMap(TrainNode::lanes) }.firstOrNull { it.id == lane }?.cards ?: emptyList()

    companion object {
        fun of(plan: Plan, sections: PlanSections = PlanSections.of(plan), locale: Locale = Locale.getDefault()): LaneTree {
            fun node(agent: PlanFleetAgent) = AgentNode(agent.id, agent.who, agent.working, agent.tokens(locale), agent.terminal)
            fun agents(lane: PlanLane): List<AgentNode> {
                val fleet = plan.agents.filter { it.laneId == lane.id }
                if (fleet.isNotEmpty()) return PlanFleet.of(fleet).agents.map(::node)
                // A runner that totals the day's agents (`agents_spend`, ov-518) lists the ones that exist: none of this lane's
                // does, whatever its records say. A lane in review between the builder and the reviewer has no agent rows.
                if (plan.agentsSpend != null) return emptyList()
                // An older runner lists none that exist: the agents the orchestrator recorded on it and has not ended.
                return lane.agents.withIndex().filter { it.value.endedAt == null }.map { (index, a) -> AgentNode("${lane.id}:$index", PlanWords.agentWho(a), true, null) }
            }
            fun laneNode(lane: PlanLane, state: String) =
                LaneNode(lane.id, lane.name, lane.heading, state, agents(lane), lane.cards)
            fun rows(rows: List<PlanSections.LaneRow>) = rows.map { laneNode(it.lane, PlanWords.status(it.lane)) }

            val groups = mutableListOf<Group>()
            if (sections.trains.isNotEmpty()) {
                groups += Group(
                    Kind.Landing, PlanSections.Kind.LANDING.title, null, false, emptyList(),
                    sections.trains.map { card ->
                        TrainNode(card.train.id, card.train.label, card.train.subtitle, card.phase.word, card.lanes.map { laneNode(it.lane, it.note?.let { note -> if (note.startsWith("Landed")) "Landed" else note } ?: "") })
                    },
                )
            }
            val waiting = sections.waiting.onYou + sections.waiting.next + sections.waiting.notRanked
            for ((kind, title, list, folded) in listOf(
                Quad(Kind.InReview, "In review", sections.inReview, false),
                Quad(Kind.Building, "Building", sections.building, false),
                Quad(Kind.Waiting, "Waiting", waiting, true),
                Quad(Kind.OnLandedTrain, "Waiting for the next landing", sections.onLandedTrain, true),
            )) {
                if (list.isNotEmpty()) groups += Group(kind, title, if (folded) "${list.size}" else null, folded, rows(list))
            }
            return LaneTree(groups)
        }

        private data class Quad(val kind: Kind, val title: String, val list: List<PlanSections.LaneRow>, val folded: Boolean)
    }
}

/** A lane's typed agent record as a row says it: "Builder · Opus". */
fun PlanWords.agentWho(a: PlanAgent): String {
    val role = when (a.role) {
        "review" -> "Reviewer"
        "fix" -> "Fixer"
        else -> "Builder"
    }
    val m = model(a.model)
    return if (m.isEmpty()) role else "$role · $m"
}
