package com.farcooler.model

/**
 * The plan in one line, on the orchestrator's tab (ov-300; Concept C of
 * .claude/agent/reports/ov-298/research-layout.md, 3.3): the orchestrator's
 * state as a mark, what needs you, what's moving and what's next. A tap opens
 * the plan as a bottom sheet. A port of AgentKit's `PlanStrip`, in the same
 * words but sentence case ("2 need you · mac-vis Building · next: plan-phones").
 */
data class PlanStrip(
    val orchestrator: Orchestrator,
    /** The orchestrator's line, for the sheet: the strip sits over the pane that shows it. */
    val line: String?,
    val needsYou: Int,
    val now: List<Pair<String, LaneState>>,
    val moreNow: Int,
    val next: String?,
) {
    enum class Orchestrator(val word: String) {
        NONE("No orchestrator"),
        STARTING("Starting"),
        WORKING("Working"),
        NEEDS_YOU("Needs you"),
        FAILED("Failed"),
        DONE("Done"),
        IDLE("Idle"),
        STOPPED("Stopped");

        /** Color only for what wants the owner. */
        val tone: Tone get() = when (this) {
            NEEDS_YOU -> Tone.ATTENTION
            FAILED, STOPPED -> Tone.FAILURE
            else -> Tone.QUIET
        }
    }

    enum class Tone { QUIET, ATTENTION, FAILURE }

    val needsYouWords: String? get() = if (needsYou > 0) "$needsYou ${if (needsYou == 1) "needs" else "need"} you" else null

    val parts: List<String>
        get() = listOfNotNull(needsYouWords) +
            now.map { (name, state) -> "$name ${PlanWords.state(state)}" } +
            listOfNotNull(if (moreNow > 0) "+$moreNow" else null, next?.let { "next: $it" })

    val isEmpty: Boolean get() = parts.isEmpty()

    val text: String get() = parts.joinToString(" · ")

    /** What TalkBack says, each thing once: "Orchestrator, working. 2 need you, mac-vis Building, next: plan-phones." */
    val accessibilityLabel: String
        get() {
            val state = when (orchestrator) {
                Orchestrator.NONE -> "No orchestrator."
                Orchestrator.NEEDS_YOU -> "Orchestrator, waiting on you."
                else -> "Orchestrator, ${orchestrator.word.lowercase()}."
            }
            return if (parts.isEmpty()) state else "$state ${parts.joinToString(", ")}."
        }

    companion object {
        /** How many Now lanes the strip names, as the Mac's does. */
        const val NOW_SHOWN = 2

        fun of(plan: Plan, needsYou: Int, orchestrator: Terminal?): PlanStrip {
            val working = plan.working
            val state = state(orchestrator)
            return PlanStrip(
                orchestrator = state, line = line(orchestrator, state), needsYou = maxOf(0, needsYou),
                now = working.take(NOW_SHOWN).map { it.name to it.state },
                moreNow = maxOf(0, working.size - NOW_SHOWN), next = plan.nextUp.firstOrNull()?.name,
            )
        }

        /** The orchestrator's state, from its pane. */
        fun state(terminal: Terminal?): Orchestrator {
            terminal ?: return Orchestrator.NONE
            when (terminal.state.lowercase()) {
                "starting" -> return Orchestrator.STARTING
                "lost", "exited", "error" -> return Orchestrator.STOPPED
            }
            if (terminal.turnDidFail) return Orchestrator.FAILED
            return when (terminal.agent) {
                AgentActivity.BLOCKED -> Orchestrator.NEEDS_YOU
                AgentActivity.WORKING -> Orchestrator.WORKING
                AgentActivity.DONE -> Orchestrator.DONE
                else -> Orchestrator.IDLE
            }
        }

        /** Its line: the question it's blocked on, what it's doing, or what it last said; never only its headline. */
        fun line(terminal: Terminal?, state: Orchestrator): String? {
            terminal ?: return null
            fun text(s: String?) = s?.trim()?.takeIf { it.isNotEmpty() }
            val headline = text(terminal.headline)
            val signal = text(terminal.signalLine)?.takeIf { it != headline }
            return when (state) {
                Orchestrator.NONE, Orchestrator.STARTING, Orchestrator.STOPPED -> null
                Orchestrator.NEEDS_YOU -> text(terminal.blockedQuestion) ?: signal
                Orchestrator.WORKING -> signal ?: text(terminal.lastSaid)
                else -> text(terminal.lastSaid)
            }
        }

        /**
         * The workspace's Needs You count, the Mac's title bar's number: the
         * runner's list, once read, plus the themes asking; before that, the
         * Needs decision column beside what's known.
         */
        fun needsYouCount(workspace: WorkspaceSummary, board: TaskBoard?, plan: Plan, list: RunnerNeedsYou?): Int {
            val items = list?.items.orEmpty().count {
                if (workspace.isImplicit) it.workspaceId == null && it.repositoryId == workspace.id else it.workspaceId == workspace.id
            }
            val asks = plan.shownThemes.count { it.ownerAsk.isNotEmpty() }
            val served = list != null && !list.derived
            return (if (served) items else (board?.waitingOnYou ?: 0) + items) + asks
        }
    }
}
