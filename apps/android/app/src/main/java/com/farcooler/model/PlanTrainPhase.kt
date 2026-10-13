package com.farcooler.model

// A train's phase (ov-503, design 4): the one reading of where a train stands that every surface shares. The runner
// derives it (`train_phase.rs`) and sends it on `BoardTrain` behind `board_train_phase`; a runner without it sends
// nothing, and phase 0 is sent as null. Both read through [PlanTrain.phaseIn], which falls back to the stored `state`
// and to what the plan knows of the train's lanes. The Android twin of AgentKit's `PlanTrainPhase.swift`.

/** Where a train stands, as a person says it. */
enum class TrainPhase(val wire: String) {
    WAITING_FOR_LANES("waiting_for_lanes"),
    INTEGRATING("integrating"),
    GATING_LOCALLY("gating_locally"),
    WAITING_FOR_CI("waiting_for_ci"),
    CI_FAILED("ci_failed"),
    READY_TO_LAND("ready_to_land"),
    LANDED("landed"),
    STALLED("stalled"),
    DROPPED("dropped"),
    UNKNOWN("unknown");

    /** The phase alone, in sentence case. */
    val word: String
        get() = when (this) {
            WAITING_FOR_LANES -> "Waiting for lanes"
            INTEGRATING -> "Integrating"
            GATING_LOCALLY -> "Gating locally"
            WAITING_FOR_CI -> "Waiting for CI"
            CI_FAILED -> "CI failed"
            READY_TO_LAND -> "Ready to land"
            LANDED -> "Landed"
            STALLED -> "Stalled"
            DROPPED -> "Dropped"
            UNKNOWN -> "Unknown"
        }

    /** The phase inside a sentence: "Train 86 waiting for CI". */
    val sentence: String
        get() = when (this) {
            CI_FAILED -> "CI failed"
            WAITING_FOR_CI -> "waiting for CI"
            else -> word.replaceFirstChar { it.lowercase() }
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
            WAITING_FOR_LANES -> 2
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
 * isn't ready (`landing`), else integrating. The fallback never says stalled or held: those need the runner's clock.
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

/** How long, coarsely: "45 min", "2 h", "3 d". */
fun PlanWords.span(ms: Long): String {
    val minutes = maxOf(0L, ms) / 60_000
    return when {
        minutes < 60 -> "$minutes min"
        minutes < 1440 -> "${minutes / 60} h"
        else -> "${minutes / 1440} d"
    }
}

/** A held train's line: "Holding for ios-overwrite · 2 h", or null when nothing holds it. */
fun PlanWords.held(train: PlanTrain, since: Long, now: Long): String? {
    if (train.heldBy.isEmpty()) return null
    return "Holding for ${train.heldBy.joinToString(", ") { it.name }} · ${span(now - since)}"
}
