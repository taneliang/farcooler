package com.farcooler.model

// A train's ladder (ov-503, design 2; ov-522): three steps, and which one a phase stands on. The Android twin of the
// Mac's `PlanLadder` (PlanTrainPhase.swift). The phone draws it compactly, as three segments and "Checking on CI · step 2
// of 3"; a long press explains each step. Waiting for the earlier landing and Waiting for main's checks to pass (the collecting train),
// Waiting for lanes, CI failed and Stalled are said in place of the step they stand on.

object PlanLadder {
    val steps = listOf("Checking here", "Checking on CI", "Landed")

    /** What each step is, one line, for a long press. */
    val explanations = listOf(
        "The full test run, on the runner.",
        "Pushed, with CI running its checks.",
        "Everything passed and it has landed on main.",
    )

    /** The step a phase is standing on, 0 to 2; 3 once everything is done. */
    fun step(phase: TrainPhase): Int = when (phase) {
        TrainPhase.WAITING_FOR_PREVIOUS_TRAIN, TrainPhase.WAITING_FOR_MAIN, TrainPhase.WAITING_FOR_LANES, TrainPhase.UNKNOWN, TrainPhase.INTEGRATING,
        TrainPhase.GATING_LOCALLY, TrainPhase.STALLED -> 0
        TrainPhase.WAITING_FOR_CI, TrainPhase.CI_FAILED -> 1
        TrainPhase.READY_TO_LAND -> 2
        TrainPhase.LANDED, TrainPhase.DROPPED -> 3
    }

    /** What step [index] says while a train stands on [current] in [phase]: the phase's own word on the step it's on. */
    fun label(index: Int, current: Int, phase: TrainPhase): String = if (index == current) phase.word else steps[index]

    /** What step [index] explains: the phase's own line on the step it's on. */
    fun explanation(index: Int, current: Int, phase: TrainPhase): String =
        if (index == current) phase.explanation else explanations[index]

    /** "Checking on CI · step 2 of 3"; just the phase once the ladder is climbed. */
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

/** [line] with the spaces inside each " · " span made non-breaking, so a line breaks between spans and never inside "1 h" (ov-515). */
fun TrainWords.unbroken(line: String): String = line.split(" · ").joinToString(" · ") { it.replace(' ', '\u00A0') }

/** "Integrator · Opus · 1.4M", or "Integrator · Opus · done". */
fun TrainWords.integrator(agent: PlanTrainAgent): String {
    val parts = mutableListOf("Integrator")
    val model = PlanWords.model(agent.model)
    if (model.isNotEmpty()) parts += model
    if (agent.spend.totalTokens > 0) parts += TaskUsageFormat.tokens(agent.spend.totalTokens)
    else if (!agent.isWorking) parts += "done"
    return parts.joinToString(" · ")
}
