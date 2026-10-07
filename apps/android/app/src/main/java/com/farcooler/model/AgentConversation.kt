package com.farcooler.model

/**
 * The conversation view of a terminal-mode claude pane on a phone (ov-374):
 * the rules it decides by, here so the JVM tests read them back. AgentKit's
 * `AgentConversation` (ov-373), with the same words, which the Mac's native view
 * keeps its own copy of.
 */
object AgentConversation {
    /**
     * Whether a runner serves the view: rows to read and a way to send. A runner
     * with rows from before `terminal.compose` gets the terminal, not a view whose
     * every send would fail. `agent_rows` is offered only while the runner's
     * projector is on, so this is also the setting.
     */
    fun served(build: DaemonBuild?): Boolean =
        build != null && build.can(Capability.AGENT_ROWS) && build.can(Capability.AGENT_COMPOSE)

    /** Whether the runner takes line breaks, images and slash commands in a message (`compose`, ov-367). */
    fun rich(build: DaemonBuild?): Boolean = build?.can(Capability.COMPOSE) == true

    /** Whether the runner presses Stop and Send now (`terminal_interrupt`, ov-368). */
    fun interrupts(build: DaemonBuild?): Boolean = build?.can(Capability.TERMINAL_INTERRUPT) == true

    /** Whether a pane is claude in a terminal, the one kind of pane the runner projects rows for. */
    fun isClaudeInATerminal(paneMode: String?, preset: String): Boolean =
        (paneMode ?: "terminal") == "terminal" && preset.startsWith("claude")

    /**
     * Whether the pane's process is there to talk to. A pane whose claude has
     * exited shows its terminal, which says how it ended, rather than a
     * conversation whose every send would be refused.
     */
    fun isRunning(state: String): Boolean = StateKind.parse(state).let { it == StateKind.RUNNING || it == StateKind.STARTING }

    /**
     * Whether a pane is offered the conversation: the runner serves it, and the
     * pane is a claude that is running in a terminal.
     *
     * Gated on the build the LAYOUT reads, [daemon] and failing that [lastDaemon],
     * never on [daemon] alone: every reconnect clears [daemon] until the new
     * link's `host` answers, and a view gated on it came down for that round trip,
     * showed the terminal and raised its keyboard, then came back (ov-373 review
     * 1). Sends still go through the runner's own gate.
     */
    fun offered(daemon: DaemonBuild?, lastDaemon: DaemonBuild?, terminal: Terminal?): Boolean =
        terminal != null && served(daemon ?: lastDaemon) &&
            isClaudeInATerminal(terminal.paneMode, terminal.preset) && isRunning(terminal.state)

    /**
     * Whether the runner settings screen shows the conversation view's switch:
     * only to the runner's host admin, on a runner that has the setting and says
     * where it stands. Client-side display only; the runner enforces it
     * (`settings.set_projector` is `host_admin`).
     */
    fun offersSetting(build: DaemonBuild?, projectorOn: Boolean?): Boolean =
        build != null && build.grantedScope == "host_admin" && build.can(Capability.PROJECTOR_SETTING) && projectorOn != null

    const val SETTING_TITLE = "Conversation view for Claude panes"
    const val SETTING_FOOTER =
        "Shows a Claude pane that runs in a terminal as a conversation you can read and reply to, " +
            "on every device that reaches this runner. Its terminal is one tap away."

    /** One key per pane (R-27). */
    fun viewKey(terminal: String): String = "nativeAgent.view.$terminal"

    /** The longest message the box takes from here (`tell.rs`'s `LONGEST_MESSAGE`). */
    const val LONGEST = 500

    /**
     * A draft as the box will take it: one line, since compose types one
     * (multi-line is `compose_into`, ov-367), so what you see is what's sent.
     */
    fun flattened(draft: String): String =
        if (draft.none { it == '\n' || it == '\r' }) draft
        else draft.replace("\r\n", " ").replace("\n", " ").replace("\r", " ")

    /**
     * Whether a message starts with a symbol claude reads as a command: a slash
     * or a bang opens its picker or its shell, which Enter would then run.
     * Commands go through the terminal until the picker is driven from here.
     */
    fun isCommand(text: String): Boolean = text.firstOrNull()?.let { it in "/!#@&$?\\" } ?: false

    /** Why a message wasn't sent, as the composer says it. */
    sealed interface SendIssue {
        /** Claude is showing a question, a menu or a panel only the terminal can draw: the Handoff row. */
        data object Handoff : SendIssue

        /** The message is one of claude's own commands that opens a panel or acts at once (`handoff`): the Handoff row. */
        data object Panel : SendIssue

        /** The terminal's box holds text of its own (R-28): refused, with Show terminal. */
        data object DraftInTerminal : SendIssue

        /** Something only words can say. */
        data class Said(val words: String) : SendIssue
    }

    /** How a failed send came back from the client core. */
    sealed interface SendFailure {
        /**
         * The runner refused it, naming why in [what] (`terminal.compose`'s words:
         * `dialog`, `draft`, `busy`, ...) and, for a refusal that isn't compose's
         * own, its error word (`scope-denied`).
         */
        data class Refused(val what: String?, val word: String? = null) : SendFailure

        /** No answer by the call's deadline, or one that couldn't be read: either way it may still be typed. */
        data object TimedOut : SendFailure

        /** The link dropped. [notSent] when the call provably never left this phone. */
        data class Lost(val notSent: Boolean) : SendFailure
    }

    /** [command] when the message was a slash command, which says which limit `images` means. */
    fun issue(failure: SendFailure, command: Boolean = false): SendIssue = when (failure) {
        // A grant that may read but not type (`read`): saying so beats
        // "wasn't sent" on every try.
        is SendFailure.Refused -> if (failure.word == "scope-denied") {
            SendIssue.Said("This device can’t send messages to this runner.")
        } else when (failure.what) {
            "prompt", "dialog" -> SendIssue.Handoff
            "handoff" -> SendIssue.Panel
            "draft" -> SendIssue.DraftInTerminal
            "typing" -> SendIssue.Said("Someone typed in the terminal in the last 15 seconds, so the message wasn’t sent. Try again once they stop.")
            "busy" -> SendIssue.Said("Claude is working and can’t take a message from here right now.")
            "too_long" -> SendIssue.Said(TOO_LONG)
            "command" -> SendIssue.Said(COMMAND_REFUSED)
            "paste_left" -> SendIssue.Said("The message didn’t land in the box as typed, so it was left there and not sent.")
            "left_at_shell" -> SendIssue.Said("Claude quit as the message was typed. It wasn’t run.")
            "unconfirmed" -> SendIssue.Said(UNCONFIRMED)
            "not_running", "not_an_agent" -> SendIssue.Said("Claude isn’t running in this pane.")
            "unfamiliar", "unproven" -> SendIssue.Said("Far Cooler can’t read this terminal’s box, so nothing was typed.")
            "unconfirmable" -> SendIssue.Said("Far Cooler can’t find Claude’s session to confirm a send, so nothing was typed.")  // casing ok: names
            "unsupported" -> SendIssue.Said("Only Claude can take a message from here. Use the terminal.")
            "images_too_large" -> SendIssue.Said(IMAGES_TOO_LARGE)
            "images" -> SendIssue.Said(if (command) COMMAND_WITH_IMAGES else TOO_MANY_IMAGES)
            "image_too_large" -> SendIssue.Said(IMAGE_TOO_LARGE)
            "backslash" -> SendIssue.Said(BACKSLASH)
            "image" -> SendIssue.Said("One of the images couldn’t be read, so nothing was sent.")
            else -> SendIssue.Said("The message wasn’t sent.")
        }
        // Never "wasn't sent" for a call that may have arrived: the runner may
        // type it yet, and a second send would go in twice.
        SendFailure.TimedOut -> SendIssue.Said(MAY_HAVE_BEEN_SENT)
        is SendFailure.Lost ->
            if (failure.notSent) SendIssue.Said("The runner isn’t connected, so the message wasn’t sent.")
            else SendIssue.Said(MAY_HAVE_BEEN_SENT)
    }

    /** Which key the runner is asked to press (ov-368). */
    enum class PaneKey { Stop, SendNow }

    /**
     * Whether claude is working on a turn, as the newest turn's row says (`Busy`).
     * Not while a dialog is up: the row says `Waiting` then, and an Esc would
     * answer the dialog No.
     */
    fun isWorking(newestTurn: AgentRow.Turn?): Boolean =
        newestTurn != null && newestTurn.outcome == null && newestTurn.activity == "Busy"

    /** The newest turn among [rows], in order. */
    fun newestTurn(rows: List<AgentRow>): AgentRow.Turn? =
        rows.lastOrNull { it.kind is AgentRow.Kind.OfTurn }?.let { (it.kind as AgentRow.Kind.OfTurn).turn }

    /**
     * What the composer says when the runner didn't press [key], or null when
     * there's nothing to say: the turn ended on its own (`idle`), or a second press
     * came too soon after the first (`too_soon`).
     */
    fun keyIssue(failure: SendFailure, key: PaneKey): SendIssue? {
        val stop = key == PaneKey.Stop
        return when (failure) {
            is SendFailure.Refused -> if (failure.word == "scope-denied") {
                SendIssue.Said("This device can’t control this runner.")
            } else when (failure.what) {
                "idle", "too_soon" -> null
                "prompt" -> SendIssue.Handoff
                "draft" -> SendIssue.DraftInTerminal
                "typing" -> SendIssue.Said("Someone is typing in the terminal. Try again in a moment.")
                "sending" -> SendIssue.Said("A message is still going in. Try again in a moment.")
                "nothing_queued" -> SendIssue.Said("Nothing is waiting in Claude’s queue.")  // casing ok: names
                "settling" -> SendIssue.Said(
                    if (stop) "Claude is starting a step. Try Stop again in a moment."  // casing ok: a button's name
                    else "Claude is starting a step. Try Send now again in a moment.",  // casing ok: a button's name
                )
                "unconfirmed" -> SendIssue.Said(
                    if (stop) "Claude didn’t confirm it stopped. It may have stopped; check the terminal before pressing again."
                    else "Claude didn’t confirm it sent the queued messages. Check the terminal.",
                )
                else -> SendIssue.Said(
                    if (stop) "Far Cooler can’t stop Claude safely from here. Use the terminal."
                    else "Far Cooler can’t send the queue safely from here. Use the terminal.",
                )
            }
            SendFailure.TimedOut, is SendFailure.Lost ->
                if (failure is SendFailure.Lost && failure.notSent) SendIssue.Said("The runner isn’t connected. Use the terminal.")
                else SendIssue.Said("The runner didn’t answer in time. Check the terminal.")
        }
    }

    /**
     * An error out of the client core, as a [SendFailure].
     *
     * A refusal carries the runner's word (`code`) or compose's `what`; a call
     * past its deadline reads as [RunnerRefusal.TIMED_OUT_WORD]. Anything else
     * that isn't a dropped link (the core closed under the call, an answer that
     * couldn't be read) has no word from any runner, and a runner may well have
     * typed the message, so it says what a timeout says rather than "wasn't
     * sent", which invites the double send (ov-373 review 1, item 4).
     */
    fun failure(error: Throwable): SendFailure = when {
        error is com.farcooler.core.DisconnectedException -> SendFailure.Lost(error.notSent)
        error is com.farcooler.core.CoreException && error.word == RunnerRefusal.TIMED_OUT_WORD -> SendFailure.TimedOut
        error is com.farcooler.core.CoreException && (error.word != null || error.what != null) ->
            SendFailure.Refused(error.what, error.word)
        else -> SendFailure.TimedOut
    }

    /** The longest with `compose` (`compose.rs`'s `LONGEST_TEXT`). */
    const val LONGEST_COMPOSED = 100_000

    /** The most images in one message (`compose.rs`'s `MOST_IMAGES`). */
    const val MOST_IMAGES = 10

    /** The longest message the box takes, where the runner has `compose` ([rich]) or doesn't. */
    fun longest(rich: Boolean): Int = if (rich) LONGEST_COMPOSED else LONGEST

    fun tooLong(rich: Boolean): String = if (rich) TOO_LONG_COMPOSED else TOO_LONG

    const val TOO_LONG = "That message is over $LONGEST characters. Shorten it, or paste it in the terminal."
    const val TOO_LONG_COMPOSED =
        "That message is over 100,000 characters. Shorten it, or paste it in the terminal." // LONGEST_COMPOSED, written out for a const
    /** The runner's `command`: a `!`, which claude's box runs in a shell, or a `/` before something that isn't a command's name. */
    const val COMMAND_REFUSED =
        "Claude would run that as a shell command or doesn’t have that command, so it wasn’t sent. Use the terminal for it."
    const val IMAGES_TOO_LARGE = "These images are too large to send together. Send fewer or smaller ones."
    const val TOO_MANY_IMAGES = "A message takes at most $MOST_IMAGES images."
    const val COMMAND_WITH_IMAGES = "A slash command can’t carry images. Send it without them."
    const val IMAGE_TOO_LARGE = "That image is too large to send. Use a smaller one."
    const val BACKSLASH =
        "Claude reads a backslash at the end as a new line, so the message wasn’t sent. Remove it, or add a word after it."
    const val UNCONFIRMED = "Claude didn’t confirm it took the message. Check the terminal before sending it again."
    const val PANEL = "This opens a panel in Claude, so it’s for the terminal."
    const val UNREADABLE_IMAGE =
        "That photo couldn’t be read. If it lives in cloud storage, open it in Photos first so it downloads."  // casing ok: a product's name
    const val COMMAND =
        "A message can’t start with a symbol Claude reads as a command, such as / or !. Use the terminal for commands."
    const val MAY_HAVE_BEEN_SENT =
        "The runner didn’t answer in time. The message may have been sent, so check the terminal before sending it again."
    const val DRAFT_IN_TERMINAL = "The terminal’s box already holds a draft. Send or clear it there first."
    const val HANDOFF = "Claude is showing something only the terminal can."
    const val STALE_UNAVAILABLE = "This session isn’t being read anymore. The terminal has it."
    const val STALE_TROUBLE = "Can’t reach the runner, so this may be out of date. Trying again…"

    /**
     * Messages claude's queue took that its transcript hasn't shown yet, once
     * the newest rows show them: as a Queued row, or as the turn each became.
     * Compared by [words], so a message with images matches its row (`[Image #1]`
     * there, `[Image]` here).
     */
    fun unsettled(queued: List<String>, newest: List<AgentRow>): List<String> {
        if (queued.isEmpty()) return queued
        val shown = newest.mapNotNull { row ->
            when (val kind = row.kind) {
                is AgentRow.Kind.OfQueued -> words(kind.queued.text)
                is AgentRow.Kind.OfTurn -> words(kind.turn.prompt)
                else -> null
            }
        }.toSet()
        return queued.filter { words(it) !in shown }
    }

    /** A Queued row's words until the transcript shows the message: each image as `[Image]`, then the text. */
    fun echo(text: String, images: Int): String {
        val trimmed = text.trim()
        return (List(images) { "[Image]" } + listOfNotNull(trimmed.ifEmpty { null })).joinToString(" ")
    }

    private val placeholder = Regex("""\[Image( #\d+)?\]""")

    /**
     * What an echo and the transcript's row for it share: how many images the
     * message has (claude's `[Image #N]`, the echo's `[Image]`), and its words
     * without them or the white space around them. So an echo of two images is
     * never taken for a row of one.
     */
    fun words(text: String): String = "${placeholder.findAll(text).count()} ${text.replace(placeholder, "").trim()}"

    // Words for rows.

    fun gap(gap: AgentRow.Gap): String =
        if (gap.count > 1) "Some of this session couldn’t be read." else "A line of this session couldn’t be read."

    /** "0:04", "1:12", "2:03:09": the format a running timer counts in, so a finished time reads like the running one did. */
    fun short(ms: Long): String {
        val seconds = maxOf(0L, ms / 1000)
        val (h, m, s) = Triple(seconds / 3600, (seconds % 3600) / 60, seconds % 60)
        fun two(n: Long) = n.toString().padStart(2, '0')
        return if (h > 0) "$h:${two(m)}:${two(s)}" else "$m:${two(s)}"
    }

    /** A subagent's type as words: `general-purpose` reads "General purpose". */
    fun agentType(raw: String): String {
        val words = raw.replace("-", " ").replace("_", " ")
        return if (words.isEmpty()) "Agent" else words.first().uppercase() + words.drop(1)
    }

    /**
     * A turn nobody typed: a background task finishing, or claude waking itself
     * (`TurnOrigin::Notification`, `System`). Drawn as a notice, never as the
     * person's message.
     */
    fun isNotice(turn: AgentRow.Turn): Boolean = turn.origin == "Notification" || turn.origin == "System"

    /** A notice turn's words, without claude's straight quotes. */
    fun noticeText(turn: AgentRow.Turn): String = turn.prompt.replace("\"", "")

    fun outcome(turn: AgentRow.Turn): String? = when (val outcome = turn.outcome) {
        null -> null
        AgentRow.Turn.Outcome.Finished -> turn.durationMs?.let { "Took ${short(it)}" } ?: "Done"
        AgentRow.Turn.Outcome.Interrupted -> "Interrupted"
        AgentRow.Turn.Outcome.Unrecorded -> "Not recorded"
        is AgentRow.Turn.Outcome.Failed -> if (outcome.detail.isEmpty()) "Failed" else "Failed: ${outcome.detail}"
        is AgentRow.Turn.Outcome.Other -> "Ended"
    }

    fun askTitle(ask: AgentRow.Ask): String {
        val by = ask.answeredBy
        if (by != null && (ask.answered || ask.held == null)) return "Answered on $by"
        if (ask.answered) return "Answered"
        return when (ask.kind) {
            "Permission" -> "Claude is asking for permission"
            "PlanExit" -> "Claude has a plan for you to review"
            else -> "Claude is asking a question"
        }
    }

    // Answering a held ask (ov-370, R-33)

    /** Whether this view can answer the ask: the runner's hook holds it and nothing has answered it yet. */
    fun answerable(ask: AgentRow.Ask): Boolean = !ask.answered && ask.held != null

    /**
     * What `terminal.agent_answer` takes for each button: a question is answered
     * with [ANSWER] and its answers; a plan with [ALLOW] (approve) or [DENY] (keep
     * planning); a permission with [ALLOW] or [DENY].
     */
    const val ALLOW = "allow"
    const val DENY = "deny"
    const val ANSWER = "answer"

    /**
     * A question's answers as claude reads them, each question's words to its
     * answer: the options picked, in the order offered, then any words typed in
     * Other, joined by ", ". Null until every question has one.
     */
    fun answers(questions: List<AgentRow.Ask.Question>, picked: Map<Int, Set<String>>, typed: Map<Int, String>): Map<String, String>? {
        if (questions.isEmpty()) return null
        val answers = LinkedHashMap<String, String>()
        questions.forEachIndexed { i, question ->
            val chosen = question.options.map { it.label }.filter { picked[i]?.contains(it) == true }
            val other = typed[i].orEmpty().trim()
            // Other is one more choice: for a single-choice question it
            // replaces the pick, as claude's own dialog has it (review 1 L1).
            val parts = when {
                other.isEmpty() -> chosen
                question.multiSelect -> chosen + other
                else -> listOf(other)
            }
            if (parts.isEmpty()) return null
            answers[question.question] = parts.joinToString(", ")
        }
        return answers
    }

    /** One pick: a single-choice question's replaces what was picked, a multi-select's toggles. */
    fun pick(label: String, question: AgentRow.Ask.Question, picked: Set<String>): Set<String> = when {
        !question.multiSelect -> setOf(label)
        label in picked -> picked - label
        else -> picked + label
    }

    /** Why an answer didn't land, by the runner's word for it. */
    fun answerIssue(what: String?, timedOut: Boolean = false): String = when {
        timedOut -> "The runner didn’t answer in time. Check the terminal before answering again."
        what == "not_held" -> "This isn’t waiting here anymore. It was answered, or only the terminal can answer it now."
        what == "not_delivered" -> "The answer didn’t reach Claude. Answer in the terminal."
        what == "answers" -> "Answer every question first."
        else -> "The answer wasn’t sent. Answer in the terminal."
    }

    fun queuedLabel(state: String): String = when (state) {
        "Withdrawn" -> "Withdrawn"
        "Sent" -> "Sent from the queue"
        else -> "Queued"
    }
}
