package com.farcooler.model

// A landing's ladder (ov-503, design 2; ov-522): three steps, and which one a phase stands on. The Android twin of the
// Mac's `PlanLadder` (PlanLandingPhase.swift). The phone draws it compactly, as three segments and "Checking on CI · step 2
// of 3"; a long press explains each step. Waiting for the earlier landing and Waiting for main's checks to pass (the collecting landing),
// Waiting for tickets, CI failed and Stalled are said in place of the step they stand on.

object PlanLadder {
    val steps = listOf("Checking here", "Checking on CI", "Landed")

    /** What each step is, one line, for a long press. */
    val explanations = listOf(
        "The full test run, on the runner.",
        "Pushed, with CI running its checks.",
        "Everything passed and it has landed on main.",
    )

    /** The step a phase is standing on, 0 to 2; 3 once everything is done. */
    fun step(phase: LandingPhase): Int = when (phase) {
        LandingPhase.WAITING_FOR_PREVIOUS_LANDING, LandingPhase.WAITING_FOR_MAIN, LandingPhase.WAITING_FOR_TICKETS, LandingPhase.UNKNOWN, LandingPhase.INTEGRATING,
        LandingPhase.GATING_LOCALLY, LandingPhase.STALLED -> 0
        LandingPhase.WAITING_FOR_CI, LandingPhase.CI_FAILED -> 1
        LandingPhase.READY_TO_LAND -> 2
        LandingPhase.LANDED, LandingPhase.DROPPED -> 3
    }

    /** What step [index] says while a landing stands on [current] in [phase]: the phase's own word on the step it's on. */
    fun label(index: Int, current: Int, phase: LandingPhase): String = if (index == current) phase.word else steps[index]

    /** What step [index] explains: the phase's own line on the step it's on. */
    fun explanation(index: Int, current: Int, phase: LandingPhase): String =
        if (index == current) phase.explanation else explanations[index]

    /** "Checking on CI · step 2 of 3"; just the phase once the ladder is climbed. */
    fun compact(phase: LandingPhase): String {
        val step = step(phase)
        return if (step >= steps.size) phase.word else "${phase.word} · step ${step + 1} of ${steps.size}"
    }
}

/** "3f78754f · Passed · 2 of 5 jobs done · 12 min · Lander · Haiku · 1.4M": the card's facts line, as the Mac says it. */
fun LandingWords.facts(card: PlanSections.LandingCard, ci: PlanCiRead?, now: Long): String {
    val parts = mutableListOf<String>()
    card.landing.pushedSha?.takeIf { it.isNotEmpty() }?.let { parts += it.take(8) }
    if (ci != null) {
        parts += ciSummary(ci)
        ciStale(ci, now)?.let { parts += it }
    } else if (card.landing.pushedSha != null) {
        parts += "CI not read yet"
    }
    if (card.since > 0) parts += PlanWords.span(now - card.since)
    card.lander?.let { parts += landerFacts(it) }
    return parts.joinToString(" · ")
}

/** [line] with the spaces inside each " · " span made non-breaking, so a line breaks between spans and never inside "1 h" (ov-515). */
fun LandingWords.unbroken(line: String): String = line.split(" · ").joinToString(" · ") { it.replace(' ', '\u00A0') }

/** "Lander · Haiku · 1.4M", or "Lander · Haiku · done". */
fun LandingWords.landerFacts(agent: PlanAgent): String {
    val parts = mutableListOf("Lander")
    val model = PlanWords.model(agent.model)
    if (model.isNotEmpty()) parts += model
    if (agent.spend.totalTokens > 0) parts += TaskUsageFormat.tokens(agent.spend.totalTokens)
    else if (!agent.working) parts += "done"
    return parts.joinToString(" · ")
}
