package com.farcooler.model

/**
 * What the owner can do with an open ruling (ov-333), and the words each action
 * leaves for the orchestrator: AgentKit's `RulingActions`, word for word,
 * because the same ruling must read the same on whichever phone is in hand.
 *
 *  - **Keep**: the owner's own mark, instant, like read state. It changes
 *    nothing in the plan, so it never reaches the orchestrator.
 *  - **Reverse**: a request to the orchestrator, sent. It does the work, then
 *    marks the ruling reversed with its commit; reversing never marks it here.
 *  - **Discuss**: a quote of the ruling left in the orchestrator's composer,
 *    for the owner to finish and send.
 *
 * A ruling's words are the orchestrator's, so they travel as [AskAboutTask]
 * makes any task's words safe: one line, no escape sequences.
 */
object RulingActions {
    /** The words left in the composer for Discuss, ending where the owner types. */
    fun discussDraft(r: PlanRuling): String = "About ruling ${r.short} (“${AskAboutTask.oneLine(r.decision)}”): "

    /** The request Reverse sends: the ruling's recorded reversal, and how to mark it done. */
    fun reverseRequest(r: PlanRuling): String {
        val short = AskAboutTask.oneLine(r.short)
        return "Please reverse ruling $short (“${AskAboutTask.oneLine(r.decision)}”). " +
            "Reversing it: ${AskAboutTask.oneLine(r.reversal)} " +
            "When it's done, mark it with `plan ruling reverse $short --sha <commit>` (no commit: leave out --sha)."
    }

    /** Where a Reverse went. */
    enum class Reversal {
        /** Sent to a chat orchestrator. */
        SENT,
        /** Typed into a terminal orchestrator's input with no Enter: the owner presses Return. */
        DRAFTED,
        /** Onto the clipboard: the runner couldn't prove the pane safe. */
        COPIED,
        /** The runner never answered in time, so it may have been typed. */
        MAYBE_DRAFTED,
        /** A chat orchestrator didn't take it. */
        FAILED,
    }

    /**
     * Send Reverse's request to the orchestrator, whichever kind of pane it is.
     * A chat pane is sent it as a typed message; a terminal pane is asked of the
     * runner, which types it with no Enter only past the gate that types an
     * answer, and anything less copies it instead.
     */
    suspend fun reverse(
        r: PlanRuling,
        isAgentPane: Boolean,
        send: suspend (String) -> Boolean,
        paste: suspend (String) -> AskAboutTask.DraftResult,
        copy: suspend (String) -> Unit,
    ): Reversal {
        val text = reverseRequest(r)
        if (isAgentPane) return if (send(text)) Reversal.SENT else Reversal.FAILED
        return when (paste(text)) {
            AskAboutTask.DraftResult.PASTED -> Reversal.DRAFTED
            AskAboutTask.DraftResult.UNKNOWN -> Reversal.MAYBE_DRAFTED
            AskAboutTask.DraftResult.DECLINED -> {
                copy(text)
                Reversal.COPIED
            }
        }
    }

    /** Leave Discuss's draft in the orchestrator's composer, or paste it into a terminal one. */
    suspend fun discuss(
        r: PlanRuling,
        isAgentPane: Boolean,
        offer: (String) -> Unit,
        paste: suspend (String) -> AskAboutTask.DraftResult,
        copy: suspend (String) -> Unit,
    ): AskAboutTask.Delivery = AskAboutTask.deliver(discussDraft(r), isAgentPane, offer, paste, copy)

    /** What Reverse asks before it sends anything (ruling R-18): the ruling named, and the reversal the request carries. */
    fun confirmTitle(r: PlanRuling): String = "Reverse ${r.short}?"

    fun confirmMessage(r: PlanRuling): String =
        "This asks the orchestrator to reverse “${AskAboutTask.oneLine(r.decision)}”. Reversing it: ${AskAboutTask.oneLine(r.reversal)}"

    /** What the screen says after a Reverse, or null when nothing needs saying. */
    fun notice(reversal: Reversal, r: PlanRuling): String? = when (reversal) {
        Reversal.SENT -> "Asked the orchestrator to reverse ${r.short}."
        Reversal.DRAFTED -> "Put the request in the orchestrator’s input. Press Return to send it." // casing ok: the key is named Return on a keyboard, and the iPhone says so too
        Reversal.COPIED -> "Copied the request to reverse ${r.short}. Paste it into the orchestrator."
        Reversal.MAYBE_DRAFTED -> "Typed, not sent. Check the orchestrator’s input for the request."
        Reversal.FAILED -> "Couldn’t reach the orchestrator. Try again."
    }
}
