package com.farcooler.model

import com.farcooler.ui.OrchestratorSeat

/**
 * Ask the orchestrator, on a task's screen (ov-241).
 *
 * The Mac's `AskOrchestrator` (ov-184) and AgentKit's `AskAboutTask`, word for
 * word, because the same task must be asked about the same way whichever phone
 * is in your hand: the orchestrator owns the task list, so a person who wants a
 * task moved, reworded, started or dropped says so to the orchestrator. This
 * puts the start of that message in its composer, naming the task, and goes
 * there; the person finishes the sentence and sends it.
 *
 * It never starts an orchestrator. With none running the control stays on the
 * screen, off, and says what to do.
 */
object AskAboutTask {
    /** The control's title. */
    const val TITLE = "Ask the orchestrator"

    /** Why the control is off, and what turns it on. */
    const val UNAVAILABLE = "Start an orchestrator to ask about this task"

    /**
     * The words left in the composer: a reference, not a copy, because the
     * orchestrator reads the task itself (`farcooler task show`). Ends where the
     * person goes on typing.
     */
    fun draft(key: String, title: String): String = "About ${oneLine(key)} (“${oneLine(title)}”): "

    /**
     * [raw] as one line that does nothing when it reaches a composer or a
     * terminal: escape sequences (a CSI such as `ESC[201~`, which would close a
     * bracketed paste, an OSC, or ESC and the character after it), every other
     * control character (^C, ^D, ^U, DEL, C1), line breaks and tabs as spaces,
     * and invisible format characters (bidi overrides, zero-width marks). Runs
     * of space become one. AgentKit's `AskAboutTask.oneLine`, rule for rule.
     */
    fun oneLine(raw: String): String {
        val out = StringBuilder()
        val points = raw.codePoints().toArray()
        var i = 0
        while (i < points.size) {
            val c = points[i++]
            when (c) {
                0x1B -> {
                    if (i >= points.size) break
                    val kind = points[i++]
                    if (kind == '['.code) {
                        // CSI: parameters, then one final byte, 0x40 to 0x7E.
                        while (i < points.size && points[i++] !in 0x40..0x7E) Unit
                    } else if (kind == ']'.code) {
                        // OSC: up to BEL, or ESC and a backslash.
                        while (i < points.size) {
                            val next = points[i++]
                            if (next == 0x07) break
                            if (next == 0x1B) { i++; break }
                        }
                    }
                }
                0x09, 0x0A, 0x0D, 0x0B, 0x0C, 0x85, 0x2028, 0x2029 -> out.append(' ')
                in 0x00..0x1F, in 0x7F..0x9F, 0xAD, in 0x200B..0x200F, in 0x202A..0x202E,
                in 0x2060..0x2064, in 0x2066..0x2069, 0xFEFF -> Unit
                else -> out.appendCodePoint(c)
            }
        }
        return out.toString().split(' ').filter { it.isNotEmpty() }.joinToString(" ")
    }

    /** Where the draft went. */
    enum class Delivery {
        /** A chat orchestrator's composer, waiting for the person. */
        COMPOSER,
        /** A terminal orchestrator's input line, pasted by the runner with no Enter. */
        PASTED,
        /** Onto the clipboard: the runner couldn't prove the pane safe. */
        COPIED,
        /**
         * The runner never answered in time, so it may have pasted after all.
         * Nothing is copied and nothing is claimed: copying as well would leave
         * the reference in the box and on the clipboard under a notice that
         * says the paste didn't happen.
         */
        MAYBE_PASTED,
    }

    /** What the runner said to a paste into a terminal orchestrator. */
    enum class DraftResult {
        PASTED,
        /** It refused, or the call never left this phone: nothing was typed. */
        DECLINED,
        /** No answer in time, or the link dropped mid-call: it may have been typed. */
        UNKNOWN;

        companion object {
            /** From a failed call, as [com.farcooler.net.WriteOutcome.of] reads one. */
            fun of(error: Throwable): DraftResult =
                if (com.farcooler.net.WriteOutcome.of(error) is com.farcooler.net.WriteOutcome.NeverSent) DECLINED
                else UNKNOWN
        }
    }

    /** What the screen says when the reference was copied instead of pasted. */
    fun copiedNotice(key: String) = "Copied a reference to ${oneLine(key)}. Paste it into the orchestrator."

    /**
     * Hand the draft to the orchestrator, whichever kind of pane it is.
     *
     * A chat pane takes it in its composer. A terminal pane (a shell running
     * claude, adopted as the orchestrator) is asked of the runner, which pastes
     * it with no Enter only past the gate that types an answer: a proven, idle
     * agent with an empty box and a known paste mode. Anything less, or any
     * failure, copies it instead. Nothing here ever presses Enter.
     */
    suspend fun deliver(
        key: String,
        title: String,
        isAgentPane: Boolean,
        offer: (String) -> Unit,
        paste: suspend (String) -> DraftResult,
        copy: suspend (String) -> Unit,
    ): Delivery {
        return deliver(draft(key, title), isAgentPane, offer, paste, copy)
    }

    /** [deliver] for any draft: the rulings' Discuss leaves its own words the same way (ov-333). */
    suspend fun deliver(
        text: String,
        isAgentPane: Boolean,
        offer: (String) -> Unit,
        paste: suspend (String) -> DraftResult,
        copy: suspend (String) -> Unit,
    ): Delivery {
        if (isAgentPane) {
            offer(text)
            return Delivery.COMPOSER
        }
        return when (paste(text)) {
            DraftResult.PASTED -> Delivery.PASTED
            DraftResult.UNKNOWN -> Delivery.MAYBE_PASTED
            DraftResult.DECLINED -> {
                copy(text.trim(' '))
                Delivery.COPIED
            }
        }
    }

    /**
     * The workspace's orchestrator, when it has one that isn't dead, and the
     * worktree it lives in. [seated] is the terminal id the runner names for
     * the workspace; an implicit workspace (a runner without workspaces) never
     * has one.
     */
    fun seat(
        workspaceId: String,
        worktrees: List<Worktree>,
        workspaces: List<WorkspaceSummary>?,
    ): OrchestratorSeat.Live? {
        val listed = workspaces?.firstOrNull { it.id == workspaceId } ?: return null
        if (listed.isImplicit) return null
        val seated = worktrees.asSequence().flatMap { it.terminals.asSequence() }
            .firstOrNull { it.isOrchestrator && it.workspace == workspaceId }?.id
            ?: listed.orchestrator
        return OrchestratorSeat.of(seated, worktrees, startedAt = null, now = 0, canStart = false)
            as? OrchestratorSeat.Live
    }
}
