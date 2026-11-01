package com.farcooler.model

import java.util.Locale

// The two-list sidebar (ov-503, design 3; ov-872): the why and the how, apart. `ThemeTree` is Themes › Cards; `TicketTree` is
// Tickets › Agents, grouped as the Plan view's sections are, so the two read in one order. They replace `OneTree`'s
// single Theme › Task › Ticket › Terminals outline. Terminals are the app's own (live panes, not plan data) and hang under
// a ticket node in the view. The Android twin of AgentKit's `PlanSidebarTrees.swift`.
//
// A ticket is a card: [ThemeTree.ticketOf] and [TicketTree.cardOf] answer both ways, and a view highlights the other
// side when one is chosen.

data class ThemeTree(val filter: OneTree.Filter, val themes: List<ThemeNode>) {
    data class CardNode(
        val task: String,
        val key: String,
        val title: String,
        val status: TaskStatus,
        /** The ticket on the line for it, shown as the trailing detail: its stage in words. */
        val ticket: String?,
        val ticketId: String?,
    ) {
        val glyph: PlanCardGlyph get() = PlanCardGlyph.of(status)
    }

    data class ThemeNode(val id: String, val name: String, val done: Int, val total: Int, val cards: List<CardNode>) {
        /** "6/10", the sidebar's short form. */
        val progress: String get() = "$done/$total"
    }

    /** The live ticket on a card, by the card's task id, or null. */
    fun ticketOf(task: String): String? = themes.firstNotNullOfOrNull { t -> t.cards.firstOrNull { it.task == task } }?.ticketId

    /** The cards a ticket is, by its task id, across themes. */
    fun cardsOf(ticket: String): List<CardNode> = themes.flatMap { it.cards }.filter { it.ticketId == ticket }

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
                    val ticket = liveTicket(plan, ref.task)
                    CardNode(ref.task, ref.key, known?.title ?: "", status, ticket?.let { PlanWords.state(it) }, ticket?.task)
                }
                if (cards.isEmpty() && !(theme.cards.isEmpty() && filter != OneTree.Filter.IN_REVIEW && theme.state == "active")) return@mapNotNull null
                ThemeNode(theme.id, theme.name, theme.counts.done, PlanWords.total(theme.counts), cards)
            }
            return ThemeTree(filter, themes)
        }

        /** The live ticket on a card. Null for a card whose ticket has landed or has none. */
        internal fun liveTicket(plan: Plan, task: String): PlanTicket? = plan.tickets.firstOrNull { it.stage.isLive && it.task == task }
    }
}

data class TicketTree(
    /** Landing (its landings inside it) first, then In review and Building, and the folded Waiting; a group with no tickets isn't listed. */
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

    data class TicketNode(
        /** The ticket's task id. */
        val id: String,
        val key: String,
        val heading: String,
        /**
         * The stage beside it: in a landing, only what matters ("Needs you", "Landed · 3f78754f"), and nothing for a
         * ticket that is simply aboard; off one, its stage.
         */
        val state: String,
        val agents: List<AgentNode>,
        val card: PlanCardRef,
    )

    /** A landing inside the Landing group: "Landing 86", what it is for, where it stands, and its tickets (ov-522). */
    data class LandingNode(
        val id: String,
        /** "Landing 86". */
        val label: String,
        /** Its title, under the label; null when it has none. */
        val subtitle: String?,
        /** "Checking on CI", the ladder step it stands on. */
        val phase: String,
        val tickets: List<TicketNode>,
    )

    sealed interface Kind {
        data object Landing : Kind
        data object InReview : Kind
        data object Building : Kind
        data object Waiting : Kind
    }

    data class Group(
        val kind: Kind,
        val title: String,
        /** A count for a folded group. */
        val detail: String?,
        /** Waiting starts folded. */
        val folded: Boolean,
        val tickets: List<TicketNode>,
        /** The landings of the Landing group, each with its tickets inside it; empty for every other group. */
        val landings: List<LandingNode> = emptyList(),
    )

    /** The card a ticket is, by its task id: what a view highlights when a ticket is chosen. */
    fun cardOf(ticket: String): PlanCardRef? =
        groups.asSequence().flatMap { it.tickets + it.landings.flatMap(LandingNode::tickets) }.firstOrNull { it.id == ticket }?.card

    companion object {
        fun of(plan: Plan, sections: PlanSections = PlanSections.of(plan), locale: Locale = Locale.getDefault()): TicketTree {
            fun node(agent: PlanAgent) = AgentNode(agent.id, agent.who, agent.working, agent.tokens(locale), agent.terminal)
            fun agents(ticket: PlanTicket): List<AgentNode> = PlanFleet.of(plan.agentsOn(ticket)).agents.map(::node)
            fun ticketNode(ticket: PlanTicket, state: String) =
                TicketNode(ticket.task, ticket.key, ticket.heading, state, agents(ticket), ticket.card)
            fun rows(rows: List<PlanSections.TicketRow>) = rows.map { ticketNode(it.ticket, PlanWords.status(it.ticket)) }

            val groups = mutableListOf<Group>()
            if (sections.landings.isNotEmpty()) {
                groups += Group(
                    Kind.Landing, PlanSections.Kind.LANDING.title, null, false, emptyList(),
                    sections.landings.map { card ->
                        LandingNode(card.landing.id, card.landing.label, card.landing.subtitle, card.phase.word, card.tickets.map { ticketNode(it.ticket, it.note?.let { note -> if (note.startsWith("Landed")) "Landed" else note } ?: "") })
                    },
                )
            }
            val waiting = sections.waiting.onYou + sections.waiting.next + sections.waiting.notRanked
            for ((kind, title, list, folded) in listOf(
                Quad(Kind.InReview, "In review", sections.inReview, false),
                Quad(Kind.Building, "Building", sections.building, false),
                Quad(Kind.Waiting, "Waiting", waiting, true),
            )) {
                if (list.isNotEmpty()) groups += Group(kind, title, if (folded) "${list.size}" else null, folded, rows(list))
            }
            return TicketTree(groups)
        }

        private data class Quad(val kind: Kind, val title: String, val list: List<PlanSections.TicketRow>, val folded: Boolean)
    }
}
