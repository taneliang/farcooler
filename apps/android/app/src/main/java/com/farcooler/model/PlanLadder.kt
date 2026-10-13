package com.farcooler.model

// A train's ladder (ov-503, design 2): five steps, and which one a phase stands on. The Android twin of the Mac's
// `PlanLadder` (PlanTrainCard.swift). The phone draws it compactly, as five segments and "Waiting for CI · step 4 of 5".

object PlanLadder {
    val steps = listOf("Picked", "Integrated", "Gated", "Waiting for CI", "Landed")

    /** The step a phase is standing on, 0 to 4; 5 once everything is done. */
    fun step(phase: TrainPhase): Int = when (phase) {
        TrainPhase.WAITING_FOR_LANES, TrainPhase.UNKNOWN -> 0
        TrainPhase.INTEGRATING, TrainPhase.STALLED -> 1
        TrainPhase.GATING_LOCALLY -> 2
        TrainPhase.WAITING_FOR_CI, TrainPhase.CI_FAILED -> 3
        TrainPhase.READY_TO_LAND -> 4
        TrainPhase.LANDED, TrainPhase.DROPPED -> 5
    }

    /** "Waiting for CI · step 4 of 5"; just the phase once the ladder is climbed. */
    fun compact(phase: TrainPhase): String {
        val step = step(phase)
        return if (step >= steps.size) phase.word else "${phase.word} · step ${step + 1} of ${steps.size}"
    }
}

/** "3f78754f · Passed · 2 of 5 jobs done · 12 min · Integrator · Opus · 1.4M": the card's facts line, as the Mac says it. */
fun TrainWords.facts(card: PlanSections.TrainCard, ci: PlanCiRead?, now: Long): String {
    val parts = mutableListOf<String>()
    card.train.pushedSha?.takeIf { it.isNotEmpty() }?.let { parts += it.take(8) }
    if (ci != null) {
        parts += ciSummary(ci)
        ciStale(ci, now)?.let { parts += it }
    } else if (card.train.pushedSha != null) {
        parts += "CI not read yet"
    }
    if (card.since > 0) parts += PlanWords.span(now - card.since)
    card.train.agent?.let { parts += integrator(it) }
    return parts.joinToString(" · ")
}

/** "Integrator · Opus · 1.4M", or "Integrator · Opus · done". */
fun TrainWords.integrator(agent: PlanTrainAgent): String {
    val parts = mutableListOf("Integrator")
    val model = PlanWords.model(agent.model)
    if (model.isNotEmpty()) parts += model
    if (agent.spend.totalTokens > 0) parts += TaskUsageFormat.tokens(agent.spend.totalTokens)
    else if (!agent.isWorking) parts += "done"
    return parts.joinToString(" · ")
}
