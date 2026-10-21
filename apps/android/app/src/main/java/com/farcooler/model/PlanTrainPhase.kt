package com.farcooler.model

// A train's phase (ov-503, design 4): the one reading of where a train stands that every surface shares. The runner
// derives it (`train_phase.rs`) and sends it on `BoardTrain` behind `board_train_phase`; a runner without it sends
// nothing, and phase 0 is sent as null. Both read through [PlanTrain.phaseIn], which falls back to the stored `state`
// and to what the plan knows of the train's lanes. The Android twin of AgentKit's `PlanTrainPhase.swift`.

/** Where a train stands, as a person says it. */
enum class TrainPhase(val wire: String) {
    WAITING_FOR_LANES("waiting_for_lanes"),

    /** The collecting train, gathering ready lanes while the train ahead lands (ov-520, ov-522). The runner sends it; no `state` reads as it. */
    WAITING_FOR_PREVIOUS_TRAIN("waiting_for_previous_train"),

    /** The collecting train with nothing ahead, held off until main's own checks pass (ov-520, proto 11). The runner sends it; no `state` reads as it. */
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
     * The phase alone, in sentence case. Every word maps to a state the runner derives; combining the lanes, the local
     * gates and the full test run are one step to a person, "Checking here" (ov-522).
     */
    val word: String
        get() = when (this) {
            WAITING_FOR_LANES -> "Waiting for lanes"
            WAITING_FOR_PREVIOUS_TRAIN -> "Waiting for the earlier landing"
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

    /** The phase inside a sentence: "Train 86 checking on CI". */
    val sentence: String
        get() = when (this) {
            CI_FAILED -> "CI failed"
            WAITING_FOR_CI -> "checking on CI"
            else -> word.replaceFirstChar { it.lowercase() }
        }

    /** One line on what is happening, for a long press. */
    val explanation: String
        get() = when (this) {
            WAITING_FOR_LANES -> "Some of its lanes aren't ready to land yet."
            WAITING_FOR_PREVIOUS_TRAIN -> "Ready lanes are collected here while the earlier landing finishes."
            WAITING_FOR_MAIN -> "Ready lanes are collected here until main's checks pass, then they land."
            INTEGRATING, GATING_LOCALLY -> "The full test run, on the runner."
            WAITING_FOR_CI -> "Pushed, with CI running its checks."
            CI_FAILED -> "A CI check failed, so it needs a fix."
            READY_TO_LAND -> "Every check passed, so it's ready to land."
            LANDED -> "Everything passed and it has landed on main."
            STALLED -> "Nothing here has moved for a while."
            DROPPED -> "It was dropped without landing."
            UNKNOWN -> "This runner didn't say where it stands."
        }

    /** How far along it is, for ordering trains furthest first. A stalled train has lost its place, and an unknown one never leads. */
    val progress: Int
        get() = when (this) {
            LANDED, DROPPED -> 8
            READY_TO_LAND -> 7
            WAITING_FOR_CI -> 6
            CI_FAILED -> 5
            GATING_LOCALLY -> 4
            INTEGRATING -> 3
            WAITING_FOR_LANES, WAITING_FOR_PREVIOUS_TRAIN, WAITING_FOR_MAIN -> 2
            STALLED -> 1
            UNKNOWN -> 0
        }

    /** The ones drawn in amber: the runner's CI failed, or nothing has moved. */
    val needsAttention: Boolean get() = this == CI_FAILED || this == STALLED

    companion object {
        /** A phase this build has no word for is [UNKNOWN], not a failed read. */
        fun parse(word: String?): TrainPhase = entries.firstOrNull { it.wire == word } ?: UNKNOWN
    }
}

/**
 * The phase to show: the runner's, else what `state` and the lanes say. Pushed, red, green, landed and dropped are
 * one-to-one with a phase; gating is gating locally; integrating is waiting for lanes while any live lane on the train
 * isn't ready (`landing`), else integrating. The fallback never says stalled: that needs the runner's clock.
 */
fun PlanTrain.phaseIn(plan: Plan): TrainPhase {
    if (phase != null && phase != TrainPhase.UNKNOWN) return phase
    return when (state) {
        TrainState.PUSHED -> TrainPhase.WAITING_FOR_CI
        TrainState.RED -> TrainPhase.CI_FAILED
        TrainState.GREEN -> TrainPhase.READY_TO_LAND
        TrainState.LANDED -> TrainPhase.LANDED
        TrainState.DROPPED -> TrainPhase.DROPPED
        TrainState.GATING -> TrainPhase.GATING_LOCALLY
        TrainState.INTEGRATING, TrainState.UNKNOWN ->
            if (plan.lanesOn(this).any { it.state.isLive && it.state != LaneState.LANDING }) TrainPhase.WAITING_FOR_LANES
            else TrainPhase.INTEGRATING
    }
}

/** When the train entered the phase to show: the runner's clock, else the state's. */
fun PlanTrain.phaseSinceIn(): Long =
    if (phase != null && phase != TrainPhase.UNKNOWN && phaseSince != null) phaseSince else stateSince

/** The plan's lanes on [train]: those it lists, plus any lane that names it, in the plan's order. */
fun Plan.lanesOn(train: PlanTrain): List<PlanLane> {
    val ids = train.lanes.map { it.lane }.toSet()
    return lanes.filter { it.id in ids || it.train.equals(train.name, ignoreCase = true) }
}

/** What a train is, for a long press where one first appears (ov-522): the Landing header and the train card. */
const val TRAIN_EXPLANATION = "Reviewed lanes land on main together, after one round of checks."

/** How long, coarsely: "45 min", "2 h", "3 d". */
fun PlanWords.span(ms: Long): String {
    val minutes = maxOf(0L, ms) / 60_000
    return when {
        minutes < 60 -> "$minutes min"
        minutes < 1440 -> "${minutes / 60} h"
        else -> "${minutes / 1440} d"
    }
}
