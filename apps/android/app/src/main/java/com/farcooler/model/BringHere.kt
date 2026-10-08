package com.farcooler.model

/**
 * Bring here (ov-369, R-28): one draft across a conversation's composer and
 * claude's own box in the terminal. The iPhone's and the Mac's `BringHere`.
 *
 * The composer's draft is the phone's own until Send; the box's is the
 * terminal's. A send needs the box empty, so a box holding text refuses it
 * (`draft`), and the composer offers Bring here, which moves the box's text into
 * the composer, or Show terminal.
 *
 * Bring here is two calls to the runner's `terminal.bring_draft`, so the text is
 * never in neither place: a read, which types nothing, then the composer takes
 * the text, then a clear, which empties the box only while it still holds
 * exactly what was read. A clear that fails leaves the text in both, and the
 * composer says so.
 */
object BringHere {
    /** Whether a pane's composer offers Bring here: claude, on a runner that serves `bring_draft`. */
    fun offered(preset: String, build: DaemonBuild?): Boolean =
        preset.startsWith("claude") && build?.can(Capability.BRING_DRAFT) == true

    /** The composer's text once the box's is brought: the box's first, since it was there first, then what the composer held. */
    fun merged(box: String, native: String): String {
        val brought = box.trim()
        val held = native.trim('\n', '\r')
        return when {
            brought.isEmpty() -> held
            held.isBlank() -> brought
            else -> brought + "\n" + held
        }
    }

    /** How a call came back: its value, or how it failed. */
    sealed interface Answer<out T> {
        data class Took<T>(val value: T) : Answer<T>
        data class Failed(val failure: AgentConversation.SendFailure) : Answer<Nothing>
    }

    /**
     * Run Bring here: [read] the box; hand its text to [place], which puts it in
     * the composer ([merged] with what the composer holds then); then [clear] the
     * box of exactly that text. Answers what the composer's line says after, null
     * for nothing.
     */
    suspend fun run(
        agent: String = "Claude",
        read: suspend () -> Answer<String>,
        place: (String) -> Unit,
        clear: suspend (String) -> Answer<Boolean>,
    ): AgentConversation.SendIssue? {
        val text = when (val answer = read()) {
            is Answer.Failed -> return issue(answer.failure, agent)
            is Answer.Took -> answer.value
        }
        // Emptied in the terminal since the send was refused: nothing to bring,
        // and the next Send finds the box free.
        if (text.isBlank()) return null
        // In the composer before the box is touched: whatever the clear does,
        // the text isn't lost.
        place(text)
        return when (val answer = clear(text)) {
            is Answer.Took -> null
            is Answer.Failed -> AgentConversation.SendIssue.DraftLeftInTerminal(leftWords(answer.failure))
        }
    }

    /** What the line says when the read was refused, and nothing moved. */
    fun issue(failure: AgentConversation.SendFailure, agent: String = "Claude"): AgentConversation.SendIssue = when (failure) {
        is AgentConversation.SendFailure.Refused -> if (failure.word == "scope-denied") {
            AgentConversation.SendIssue.Said("This device can’t change what’s in this runner’s terminals.")
        } else when (failure.what) {
            "pasted" -> AgentConversation.SendIssue.Said(
                "The terminal’s box holds a pasted block or an image Far Cooler can’t read. Use the terminal.",  // casing ok: names
            )
            "too_tall" -> AgentConversation.SendIssue.Said("The terminal’s draft is too long to bring here whole. Use the terminal.")
            "cursor" -> AgentConversation.SendIssue.Said(
                "The cursor in the terminal’s box isn’t at the end, so its draft wasn’t moved. Use the terminal.",
            )
            "typing", "sending" -> AgentConversation.SendIssue.Said("Someone is typing in the terminal. Try again in a moment.")
            "prompt", "dialog" -> AgentConversation.SendIssue.Handoff
            "unsupported" -> AgentConversation.SendIssue.Said("$agent’s draft can’t be brought here. Use the terminal.")
            "not_running", "not_an_agent" -> AgentConversation.SendIssue.Said("$agent isn’t running in this pane.")
            else -> AgentConversation.SendIssue.Said("Far Cooler can’t read the terminal’s box. Use the terminal.")  // casing ok: names
        }
        AgentConversation.SendFailure.TimedOut -> AgentConversation.SendIssue.Said("The runner didn’t answer in time. Check the terminal.")
        is AgentConversation.SendFailure.Lost ->
            if (failure.notSent) AgentConversation.SendIssue.Said("The runner isn’t connected. Use the terminal.")
            else AgentConversation.SendIssue.Said("The runner didn’t answer in time. Check the terminal.")
    }

    /** What the line says when the box's text is here but the clear didn't go: the text is in both places. */
    fun leftWords(failure: AgentConversation.SendFailure): String = when {
        failure is AgentConversation.SendFailure.Refused && failure.what == "partly" ->
            "The draft is here, and part of it is still in the terminal’s box. Clear it there before sending."
        failure is AgentConversation.SendFailure.Refused && (failure.what == "typing" || failure.what == "changed") ->
            "The draft is here, but someone changed the terminal’s box, so it was left there. Clear it there before sending."
        failure == AgentConversation.SendFailure.TimedOut ||
            (failure is AgentConversation.SendFailure.Lost && !failure.notSent) ->
            "The draft is here. The runner didn’t answer in time, so check the terminal’s box before sending."
        else -> "The draft is here, but it’s still in the terminal’s box too. Clear it there before sending."
    }
}
