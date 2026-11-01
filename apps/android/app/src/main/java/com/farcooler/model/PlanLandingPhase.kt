package com.farcooler.model

// A landing's phase (ov-503, design 4; ov-872): the one reading of where a landing stands that every surface shares. The
// runner derives it (`landing_phase.rs`) and sends it on `Landing`; a runner without it sends nothing, and phase 0 is
// sent as null. Both read through [PlanLanding.phaseIn], which falls back to the stored `state`. The Android twin of
// AgentKit's `PlanLandingPhase.swift`.

/** Where a landing stands, as a person says it. */
enum class LandingPhase(val wire: String) {
    WAITING_FOR_TICKETS("waiting_for_tickets"),

    /** The collecting landing, gathering ready tickets while the landing ahead finishes (ov-520, ov-522). The runner sends it; no `state` reads as it. */
    WAITING_FOR_PREVIOUS_LANDING("waiting_for_previous_landing"),

    /** The collecting landing with nothing ahead, held off until main's own checks pass (ov-520, proto 11). The runner sends it; no `state` reads as it. */
    WAITING_FOR_MAIN("waiting_for_main"),
    INTEGRATING("integrating"),
    GATING_LOCALLY("gating_locally"),
    WAITING_FOR_CI("waiting_for_ci"),
    CI_FAILED("ci_failed"),
    READY_TO_LAND("ready_to_land"),
    LANDED("landed"),
    STALLED("stalled"),
    DROPPED("dropped"),
    UNKNOWN("unknown");

    /**
     * The phase alone, in sentence case. Every word maps to a state the runner derives; combining the tickets, the local
     * gates and the full test run are one step to a person, "Checking here" (ov-522).
     */
    val word: String
        get() = when (this) {
            WAITING_FOR_TICKETS -> "Waiting for tickets"
            WAITING_FOR_PREVIOUS_LANDING -> "Waiting for the earlier landing"
            WAITING_FOR_MAIN -> "Waiting for main's checks to pass"
            INTEGRATING, GATING_LOCALLY -> "Checking here"
            WAITING_FOR_CI -> "Checking on CI"
            CI_FAILED -> "CI failed"
            READY_TO_LAND -> "Ready to land"
            LANDED -> "Landed"
            STALLED -> "Stalled"
            DROPPED -> "Dropped"
            UNKNOWN -> "Unknown"
        }

    /** The phase inside a sentence: "Landing 86 checking on CI". */
    val sentence: String
        get() = when (this) {
            CI_FAILED -> "CI failed"
            WAITING_FOR_CI -> "checking on CI"
            else -> word.replaceFirstChar { it.lowercase() }
        }

    /** One line on what is happening, for a long press. */
    val explanation: String
        get() = when (this) {
            WAITING_FOR_TICKETS -> "Some of its tickets aren't ready to land yet."
            WAITING_FOR_PREVIOUS_LANDING -> "Ready tickets are collected here while the earlier landing finishes."
            WAITING_FOR_MAIN -> "Ready tickets are collected here until main's checks pass, then they land."
            INTEGRATING, GATING_LOCALLY -> "The full test run, on the runner."
            WAITING_FOR_CI -> "Pushed, with CI running its checks."
            CI_FAILED -> "A CI check failed, so it needs a fix."
            READY_TO_LAND -> "Every check passed, so it's ready to land."
            LANDED -> "Everything passed and it has landed on main."
            STALLED -> "Nothing here has moved for a while."
            DROPPED -> "It was dropped without landing."
            UNKNOWN -> "This runner didn't say where it stands."
        }

    /** How far along it is, for ordering landings furthest first. A stalled landing has lost its place, and an unknown one never leads. */
    val progress: Int
        get() = when (this) {
            LANDED, DROPPED -> 8
            READY_TO_LAND -> 7
            WAITING_FOR_CI -> 6
            CI_FAILED -> 5
            GATING_LOCALLY -> 4
            INTEGRATING -> 3
            WAITING_FOR_TICKETS, WAITING_FOR_PREVIOUS_LANDING, WAITING_FOR_MAIN -> 2
            STALLED -> 1
            UNKNOWN -> 0
        }

    /** The ones drawn in amber: the runner's CI failed, or nothing has moved. */
    val needsAttention: Boolean get() = this == CI_FAILED || this == STALLED

    companion object {
        /** A phase this build has no word for is [UNKNOWN], not a failed read. */
        fun parse(word: String?): LandingPhase = entries.firstOrNull { it.wire == word } ?: UNKNOWN
    }
}

/**
 * The phase to show: the runner's, else what `state` and the tickets say. Pushed, red, green, landed and dropped are
 * one-to-one with a phase; gating is gating locally; open is waiting for tickets while any ticket on the landing
 * isn't ready (`landing`), else integrating. The fallback never says stalled: that needs the runner's clock.
 */
fun PlanLanding.phaseIn(plan: Plan): LandingPhase {
    if (phase != null && phase != LandingPhase.UNKNOWN) return phase
    return when (state) {
        LandingState.PUSHED -> LandingPhase.WAITING_FOR_CI
        LandingState.RED -> LandingPhase.CI_FAILED
        LandingState.GREEN -> LandingPhase.READY_TO_LAND
        LandingState.LANDED -> LandingPhase.LANDED
        LandingState.DROPPED -> LandingPhase.DROPPED
        LandingState.GATING -> LandingPhase.GATING_LOCALLY
        LandingState.OPEN, LandingState.UNKNOWN ->
            if (plan.ticketsOn(this).any { it.stage.isLive && it.stage != TicketStage.LANDING }) LandingPhase.WAITING_FOR_TICKETS
            else LandingPhase.INTEGRATING
    }
}

/** When the landing entered the phase to show: the runner's clock, else the state's. */
fun PlanLanding.phaseSinceIn(): Long =
    if (phase != null && phase != LandingPhase.UNKNOWN && phaseSince != null) phaseSince else stateSince

/** The plan's tickets on [landing]: those it lists, plus any ticket that names its number, in the plan's order. */
fun Plan.ticketsOn(landing: PlanLanding): List<PlanTicket> {
    val tasks = landing.tickets.map { it.task }.toSet()
    return tickets.filter { it.task in tasks || it.landing == landing.number }
}

/** What a landing is, for a long press where one first appears (ov-522): the Landing header and the landing card. */
const val LANDING_EXPLANATION = "Reviewed tickets land on main together, after one round of checks."

/** How long, coarsely: "45 min", "2 h", "3 d". */
fun PlanWords.span(ms: Long): String {
    val minutes = maxOf(0L, ms) / 60_000
    return when {
        minutes < 60 -> "$minutes min"
        minutes < 1440 -> "${minutes / 60} h"
        else -> "${minutes / 1440} d"
    }
}
