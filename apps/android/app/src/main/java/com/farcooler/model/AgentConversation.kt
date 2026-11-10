package com.farcooler.model

import com.farcooler.core.NativeClient
import com.farcooler.core.coreIsAvailable
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * The conversation view of a terminal-mode claude or codex pane on a phone (ov-374, ov-416):
 * the rules it decides by, here so the JVM tests read them back. AgentKit's
 * `AgentConversation` (ov-373), with the same words, which the Mac's native view
 * keeps its own copy of.
 */
object AgentConversation {
    /**
     * Whether a runner serves the view: rows to read and a way to send. A runner
     * with rows from before `terminal.compose` gets the terminal, not a view whose
     * every send would fail. Every runner from ov-830 offers both; one from before
     * offers `agent_rows` only while its projector setting is on.
     */
    fun served(build: DaemonBuild?): Boolean =
        build != null && build.can(Capability.AGENT_ROWS) && build.can(Capability.AGENT_COMPOSE)

    /** Whether the runner takes line breaks, images and slash commands in a message (`compose`, ov-367). */
    fun rich(build: DaemonBuild?): Boolean = build?.can(Capability.COMPOSE) == true

    /** Whether the runner presses Stop and Send now (`terminal_interrupt`, ov-368). */
    fun interrupts(build: DaemonBuild?): Boolean = build?.can(Capability.TERMINAL_INTERRUPT) == true

    /** Whether the runner reads and clears claude's box for Bring here (`bring_draft`, ov-369). */
    fun bring(build: DaemonBuild?): Boolean = build?.can(Capability.BRING_DRAFT) == true

    /** Whether the runner serves a subagent's own rows (`subagent_rows`, ov-453, ov-641). */
    fun subagentRows(build: DaemonBuild?): Boolean = build?.can(Capability.SUBAGENT_ROWS) == true

    /** Whether the runner serves a prompt's images (`agent_images`, ov-454). */
    fun images(build: DaemonBuild?): Boolean = build?.can(Capability.AGENT_IMAGES) == true

    /**
     * Whether a pane is an agent in a terminal the runner projects rows for and
     * composes into: claude, or codex where the runner says it does (`codex_view`,
     * ov-416). A runner from before projected no codex rows and refused every send
     * into one.
     */
    fun isAgentInATerminal(paneMode: String?, preset: String, build: DaemonBuild?): Boolean =
        (paneMode ?: "terminal") == "terminal" &&
            (preset.startsWith("claude") || (preset.startsWith("codex") && build?.can(Capability.CODEX_VIEW) == true))

    /** The agent's name, as the conversation's words say it. */
    fun agentName(preset: String): String = if (preset.startsWith("codex")) "Codex" else "Claude"

    /** Whether the runner presses Stop and Send now in this agent's pane: claude's keys alone. */
    fun pressesKeys(preset: String): Boolean = preset.startsWith("claude")

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
            isAgentInATerminal(terminal.paneMode, agent(terminal), daemon ?: lastDaemon) && isRunning(terminal.state)

    /**
     * The agent a pane runs, which the view is offered by (ov-443): the one the
     * runner sees running there, else what it was launched as, else, from a
     * runner too old to say either, the label. Never the label where either is
     * there: claude names it after its session, so a pane offered by it was never
     * offered, and a claude typed into a shell was launched as `shell`.
     */
    fun agent(running: String?, program: String?, preset: String): String = running ?: program ?: preset

    fun agent(terminal: Terminal): String = agent(terminal.runningAgent, terminal.program, terminal.preset)

    /**
     * Why a pane isn't offered the conversation view (ov-443), AgentKit's `Unavailable` with its words,
     * and the check it failed as `farcooler terminal conversation-check` names it (ov-830). There is no
     * setting: the view is offered wherever a runner serves it.
     */
    enum class Unavailable(val sentence: String, val check: String, val isAboutTheRunner: Boolean = true) {
        RUNNER_NEEDS_UPDATE("This runner needs an update to show the conversation.", "Runner version"),
        NOT_AN_AGENT("This isn’t a Claude or Codex pane.", "Pane", isAboutTheRunner = false),
        NOT_RUNNING("The agent in this pane isn’t running.", "Agent running", isAboutTheRunner = false),
        UNREACHABLE("Far Cooler can’t reach this runner right now.", "Connection"),
        ;

        /** The reason with its check, as a dimmed switch says it. */
        val note: String get() = "$check: $sentence"
    }

    /**
     * Why the pane isn't offered the view, null where it is. The pane first, so a
     * shell is never told about its runner; then the runner: [build] null while
     * there's no link. A runner without rows and compose needs an update: every
     * runner from ov-830 offers both, and one from before offers rows only while
     * its projector setting is on, which an update retires. The phone doesn't turn
     * that setting on for it: that brought back the always-on projector, and a
     * phone without full access can't.
     */
    fun unavailable(paneMode: String?, agent: String, running: Boolean, build: DaemonBuild?): Unavailable? {
        val claude = agent.startsWith("claude")
        val codex = agent.startsWith("codex")
        if ((paneMode ?: "terminal") != "terminal" || !(claude || codex)) return Unavailable.NOT_AN_AGENT
        if (!running) return Unavailable.NOT_RUNNING
        if (build == null) return Unavailable.UNREACHABLE
        if (!served(build)) return Unavailable.RUNNER_NEEDS_UPDATE
        if (codex && !build.can(Capability.CODEX_VIEW)) return Unavailable.RUNNER_NEEDS_UPDATE
        return null
    }

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

        /** The terminal's box holds text of its own (R-28): refused, with Bring here where it's offered ([BringHere]), and Show terminal. */
        data object DraftInTerminal : SendIssue

        /** Bring here put the box's text in the composer but couldn't clear the box: the text is in both, as [words] says. Show terminal. */
        data class DraftLeftInTerminal(val words: String) : SendIssue

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
    fun issue(failure: SendFailure, command: Boolean = false, agent: String = "Claude"): SendIssue = when (failure) {
        is SendFailure.Refused -> {
            val send = ConversationWords.table(agent)?.send
            val found = failure.word?.let { send?.get("word:$it") }
                ?: (if (command) send?.get("${failure.what ?: ""}:command") else null)
                ?: send?.get(failure.what ?: "") ?: send?.get("")
            found?.let(::sendIssue) ?: SendIssue.Said("")
        }
        // Never "wasn't sent" for a call that may have arrived: a second send would go in twice.
        SendFailure.TimedOut -> SendIssue.Said(MAY_HAVE_BEEN_SENT)
        is SendFailure.Lost ->
            if (failure.notSent) SendIssue.Said(ConversationWords.phrase("not_connected")) else SendIssue.Said(MAY_HAVE_BEEN_SENT)
    }

    private fun sendIssue(found: ConversationWords.Issue): SendIssue? = when (found.kind) {
        ConversationWords.Kind.Handoff -> SendIssue.Handoff
        ConversationWords.Kind.Panel -> SendIssue.Panel
        ConversationWords.Kind.Draft -> SendIssue.DraftInTerminal
        ConversationWords.Kind.Said -> SendIssue.Said(found.text ?: "")
        ConversationWords.Kind.Quiet -> null
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

    /**
     * The prompt the composer offers as its placeholder (ov-409): the one claude's
     * own box shows after a turn, carried on the newest turn's row. Only while the
     * rows are live, the agent isn't working or holding a dialog (its box then shows
     * hints, not predictions), and nothing is typed. A draft the person may take,
     * never a message: nothing here sends.
     */
    fun suggestion(newestTurn: AgentRow.Turn?, draft: String, stale: Boolean): String? {
        if (stale || newestTurn == null || newestTurn.activity == "Busy" || newestTurn.activity == "Waiting") return null
        if (draft.isNotEmpty()) return null
        return newestTurn.suggestion?.trim()?.takeIf { it.isNotEmpty() }
    }

    /**
     * claude's generic `Try "…"` example, shown as the composer's placeholder in
     * place of "Message Claude" (ov-409): only while the rows are live and nothing
     * is typed. A hint: Tab and a tap do not take it, as in claude.
     */
    fun hint(rows: List<AgentRow>, draft: String, stale: Boolean): String? {
        if (stale || draft.isNotEmpty()) return null
        val kind = rows.lastOrNull { it.id == AgentRow.HINT_ID }?.kind as? AgentRow.Kind.OfHint
        return kind?.hint?.text?.trim()?.takeIf { it.isNotEmpty() }
    }

    /** The newest turn among [rows], in order. */
    fun newestTurn(rows: List<AgentRow>): AgentRow.Turn? =
        rows.lastOrNull { it.kind is AgentRow.Kind.OfTurn }?.let { (it.kind as AgentRow.Kind.OfTurn).turn }

    /**
     * What the composer says when the runner didn't press [key], or null when
     * there's nothing to say: the turn ended on its own (`idle`), or a second press
     * came too soon after the first (`too_soon`).
     */
    fun keyIssue(failure: SendFailure, key: PaneKey): SendIssue? = when (failure) {
        is SendFailure.Refused -> {
            val words = ConversationWords.table()
            val table = if (key == PaneKey.Stop) words?.stop else words?.sendNow
            val found = failure.word?.let { table?.get("word:$it") } ?: table?.get(failure.what ?: "") ?: table?.get("")
            found?.let(::sendIssue)
        }
        SendFailure.TimedOut, is SendFailure.Lost ->
            if (failure is SendFailure.Lost && failure.notSent) SendIssue.Said(ConversationWords.phrase("key_not_connected"))
            else SendIssue.Said(ConversationWords.phrase("key_timed_out"))
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

    /** The sentence for refusal word [what], as [issue] says it. */
    fun refused(what: String, agent: String = "Claude"): String = ConversationWords.table(agent)?.send?.get(what)?.text ?: ""

    val TOO_LONG: String get() = refused("too_long")
    val TOO_LONG_COMPOSED: String get() = ConversationWords.phrase("too_long_composed")
    fun handoff(agent: String): String = ConversationWords.phrase("handoff", agent)
    fun panel(agent: String): String = ConversationWords.phrase("panel", agent)
    val TOO_MANY_IMAGES: String get() = refused("images")
    const val UNREADABLE_IMAGE =
        "That photo couldn’t be read. If it lives in cloud storage, open it in Photos first so it downloads."  // casing ok: a product's name
    val COMMAND: String get() = ConversationWords.phrase("command")
    val MAY_HAVE_BEEN_SENT: String get() = ConversationWords.phrase("may_have_been_sent")
    val DRAFT_IN_TERMINAL: String get() = ConversationWords.phrase("draft_in_terminal")

    /** [DRAFT_IN_TERMINAL], where Bring here is offered beside Show terminal. */
    val DRAFT_IN_TERMINAL_BRING: String get() = ConversationWords.phrase("draft_in_terminal_bring")
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

    fun gap(gap: AgentRow.Gap): String = ConversationWords.phrase(if (gap.count > 1) "gap" else "gap_one")

    /** "0:04", "1:12", "2:03:09": the core's `conversation_words::short`. */
    fun short(ms: Long): String = ConversationWords.item("ms", "$ms").orEmpty()

    /** What a subagent row says of its work: "1 tool", "14 tools", or null while the count is unknown (0 is how a missing count reads; ov-490). */
    fun toolsLabel(count: Int): String? =
        if (count <= 0) null else ConversationWords.phrase(if (count == 1) "tools_one" else "tools", fill = mapOf("n" to "$count"))

    /** A subagent's type as words: `general-purpose` reads "General purpose". */
    fun agentType(raw: String): String = ConversationWords.item("agent_type", JsonPrimitive(raw).toString()) ?: raw

    /**
     * A turn nobody typed: a background task finishing, or claude waking itself
     * (`TurnOrigin::Notification`, `System`). Drawn as a notice, never as the
     * person's message.
     */
    fun isNotice(turn: AgentRow.Turn): Boolean = ConversationWords.item("origin", JsonPrimitive(turn.origin).toString()) == "true"

    /** The lines a report or a message shows before Show more (ov-474). */
    const val HARNESS_LINES = 4

    /** A turn claude wrote for itself: its own row, never the person's message. */
    fun isHarness(turn: AgentRow.Turn): Boolean = turn.harness != null

    /**
     * The row's heading: "Report from <the agent's description>", else from a
     * short id. A status line is its own words; every note has the same heading.
     */
    fun harnessTitle(harness: AgentRow.Harness): String =
        if (harness.kind == "Status") harness.title
        else ConversationWords.phrase("harness", by = harness.kind, fill = mapOf("name" to harnessAgentName(harness)))

    /** What the agent is called: its launch's description, else "agent" and its id's first characters, else "a subagent". */
    fun harnessAgentName(harness: AgentRow.Harness): String {
        harness.source?.takeIf { it.isNotEmpty() }?.let { return it }
        val id = harness.agentId?.takeIf { it.isNotEmpty() } ?: return ConversationWords.phrase("harness_a_subagent")
        return ConversationWords.phrase("harness_agent", fill = mapOf("id" to id.take(6)))
    }

    /** A notice turn's words, without claude's straight quotes. */
    fun noticeText(turn: AgentRow.Turn): String = turn.prompt.replace("\"", "")

    fun outcome(turn: AgentRow.Turn): String? {
        fun say(name: String, fill: Map<String, String> = emptyMap()) = ConversationWords.phrase("outcome_$name", fill = fill)
        return when (val outcome = turn.outcome) {
            null -> null
            AgentRow.Turn.Outcome.Finished -> turn.durationMs?.let { say("took", mapOf("time" to short(it))) } ?: say("done")
            AgentRow.Turn.Outcome.Interrupted -> say("interrupted")
            AgentRow.Turn.Outcome.Unrecorded -> say("unrecorded")
            is AgentRow.Turn.Outcome.Failed -> if (outcome.detail.isEmpty()) say("failed") else say("failed_detail", mapOf("detail" to outcome.detail))
            is AgentRow.Turn.Outcome.Other -> say("ended")
        }
    }

    /**
     * [viewer] is who this device is on the runner (`member.list`'s `you`: "" for the owner, a member's handle, null if
     * not told). On a runner with members an answered ask says who answered ([AgentRow.Ask.answeredSentence], ov-746);
     * a solo runner, and a viewer not yet told, read as they always did.
     */
    fun askTitle(ask: AgentRow.Ask, agent: String = "Claude", viewer: String? = null): String {
        if (ask.answered || ask.held == null) ask.answeredSentence(viewer)?.let { return it }
        val by = ask.answeredBy
        if (by != null && (ask.answered || ask.held == null)) return ConversationWords.phrase("ask_answered_on", fill = mapOf("by" to by))
        if (ask.answered) return ConversationWords.phrase("ask_answered")
        return ConversationWords.phrase("ask", agent, by = ask.kind)
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
    fun answerIssue(what: String?, timedOut: Boolean = false, agent: String = "Claude"): String {
        val answer = ConversationWords.table(agent)?.answer
        return (if (timedOut) answer?.get("timed_out") else answer?.get(what ?: "")) ?: answer?.get("") ?: ""
    }

    /** A Queued row's label. `InBox` is a message in claude's box that the runner sends when the turn ends (ov-827). */
    fun queuedLabel(state: String): String = ConversationWords.phrase("queued", by = state)

    // MARK: - Grouped tool runs (ov-452, ov-459)

    /** What a row is to the grouping: a tool call, thinking (which a group takes in only between two calls), or anything else, which ends a run. */
    enum class GroupRole { TOOL, THINKING, OTHER }

    fun groupRole(kind: AgentRow.Kind): GroupRole = when (kind) {
        is AgentRow.Kind.OfTool -> GroupRole.TOOL
        is AgentRow.Kind.OfThinking -> GroupRole.THINKING
        else -> GroupRole.OTHER
    }

    /**
     * What a conversation view draws, item by item: a row, or a run of tool calls
     * folded into one line that opens to them, as claude's own "Ran 3 shell
     * commands" does. AgentKit's `ConversationItem`.
     */
    sealed interface Item {
        val id: String

        data class Row(override val id: String) : Item

        /** Two or more tool calls in a row, with the thinking between them: the ids of every row it holds, oldest first. */
        data class Tools(override val id: String, val rows: List<String>) : Item
    }

    /**
     * [ids] as items: each run of two or more tool calls, with the thinking rows
     * between them, is one [Item.Tools]; everything else, a tool call on its own
     * included, is a [Item.Row]. Text, a message, a subagent, a question or a task
     * list ends a run. Thinking before a run's first call or after its last stays
     * a row of its own.
     */
    fun items(ids: List<String>, role: (String) -> GroupRole): List<Item> {
        val items = mutableListOf<Item>()
        // The open run: rows since its first call, and how many are calls.
        val run = mutableListOf<String>()
        var calls = 0
        // Thinking seen since the run's last call, not yet known to be inside.
        val trailing = mutableListOf<String>()

        fun close() {
            val first = run.firstOrNull()
            if (calls >= 2 && first != null) items.add(Item.Tools("tools:$first", run.toList())) else run.mapTo(items) { Item.Row(it) }
            trailing.mapTo(items) { Item.Row(it) }
            run.clear()
            trailing.clear()
            calls = 0
        }

        for (id in ids) {
            when (role(id)) {
                GroupRole.TOOL -> {
                    run.addAll(trailing)
                    trailing.clear()
                    run.add(id)
                    calls++
                }
                GroupRole.THINKING -> if (run.isEmpty()) items.add(Item.Row(id)) else trailing.add(id)
                GroupRole.OTHER -> {
                    close()
                    items.add(Item.Row(id))
                }
            }
        }
        close()
        return items
    }

    /**
     * A group's line: what its calls did, and how many. "Ran 3 commands" when
     * every call is a shell command, "Read 2 files", "Edited 4 files", "Ran 2
     * searches", else "Used 5 tools"; in the present tense while any is still
     * running.
     */
    fun groupTitle(tools: List<AgentRow.Tool>): String {
        val names = tools.map { it.name }.toSet()
        val group = ConversationWords.table()?.groups?.firstOrNull { it.names.isEmpty() || it.names.containsAll(names) } ?: return ""
        val line = if (tools.any { it.status == AgentRow.Status.Running }) group.running else group.done
        return line.replace("{n}", "${tools.size}")
    }

    /**
     * What a group's calls were for, from their own summaries: a command's
     * description, a file's name. Each said once, in order, joined by commas; a
     * view cuts it to fit.
     */
    fun groupPurpose(tools: List<AgentRow.Tool>): String {
        val phrases = LinkedHashSet<String>()
        for (tool in tools) {
            val summary = tool.presentationSummary.trim()
            phrases.add(if (tool.operation != null) summary else if (summary.isEmpty()) tool.name else purpose(summary))
        }
        return phrases.joinToString(", ")
    }

    /**
     * One summary as a phrase inside a sentence: a path as its file's name, and a
     * sentence's capital lowered ("Map the code" reads "map the code"), unless the
     * word is all capitals or a name like "README".
     */
    internal fun purpose(summary: String): String {
        if ((summary.startsWith("/") || summary.startsWith("~/")) && !summary.contains(" ")) {
            return summary.trimEnd('/').substringAfterLast('/').ifEmpty { "/" }
        }
        if (summary.length > 1 && summary[0].isUpperCase() && summary[1].isLowerCase()) {
            return summary[0].lowercaseChar() + summary.drop(1)
        }
        return summary
    }

    /** A group's state: running while any call is, failed if any failed, else done. */
    fun groupStatus(tools: List<AgentRow.Tool>): AgentRow.Status = when {
        tools.any { it.presentationStatus == AgentRow.Status.Running } -> AgentRow.Status.Running
        tools.any { it.presentationStatus == AgentRow.Status.Failed } -> AgentRow.Status.Failed
        tools.any { it.operation?.lifecycle == "unconfirmed" } -> AgentRow.Status.Ended("Outcome unconfirmed")
        else -> AgentRow.Status.Done
    }

    fun recognizedGroupLabel(tools: List<AgentRow.Tool>): String? {
        if (tools.none { it.operation != null }) return null
        val words = when (groupStatus(tools)) {
            AgentRow.Status.Running -> "Commands running"
            AgentRow.Status.Done -> "Commands completed"
            AgentRow.Status.Failed -> "Command failed or declined"
            else -> "Outcome unconfirmed"
        }
        return "Recognized commands · $words"
    }

    // Long messages

    /**
     * The lines a long message shows before Show more. Whether it runs past them
     * is measured at the width it's drawn at, never guessed from its length.
     */
    const val COLLAPSED_LINES = 6

    // Where a message came from

    /** What a turn's footer says about where its prompt came from, if anything: a message typed while the agent worked. */
    fun originNote(turn: AgentRow.Turn): String? = if (turn.origin == "Queued") ConversationWords.phrase("origin_queued") else null

    /** A scheduled task's firing (`CronCreate`, `/loop`): drawn as one, with its prompt folded, never as the person's message. */
    fun isScheduled(turn: AgentRow.Turn): Boolean = turn.origin == "Scheduled"

    val SCHEDULED_TASK: String get() = ConversationWords.phrase("scheduled_task")

    // Task lists

    /** "2 of 5 done". */
    fun taskProgress(tasks: AgentRow.Tasks): String = ConversationWords.phrase(
        "tasks_progress", fill = mapOf("done" to "${tasks.items.count { it.status == "Completed" }}", "n" to "${tasks.items.size}"),
    )

    /** A task's state, as TalkBack says it. */
    fun taskState(item: AgentRow.TaskItem): String = ConversationWords.phrase("task", by = item.status)

    // Prompt images (ov-454, ov-458)

    private val imagePlaceholder = Regex("""[ \t]?\[Image #\d+]""")

    /**
     * [prompt] without its first [shown] `[Image #N]` placeholders, and the space
     * before each; the rest, and every other word, as they are.
     */
    fun withoutPlaceholders(prompt: String, shown: Int): String {
        if (shown <= 0) return prompt
        val matches = imagePlaceholder.findAll(prompt).take(shown).toList()
        val out = StringBuilder(prompt)
        for (match in matches.asReversed()) out.delete(match.range.first, match.range.last + 1)
        return out.toString().trim()
    }
}

/**
 * An agent conversation's words, from the shared core's `conversation_words` (ov-1049, ruling R-112) through the
 * plan view's bridge, in Material's case: one table per agent, asked once. Empty where the core isn't in this build.
 */
object ConversationWords {
    @Serializable
    enum class Kind {
        @SerialName("handoff") Handoff, @SerialName("panel") Panel, @SerialName("draft") Draft,
        @SerialName("said") Said, @SerialName("quiet") Quiet,
    }

    @Serializable
    data class Issue(val kind: Kind, val text: String? = null)

    @Serializable
    data class Group(val names: List<String>, val running: String, val done: String)

    @Serializable
    data class Table(
        val send: Map<String, Issue>, val stop: Map<String, Issue>, @SerialName("send_now") val sendNow: Map<String, Issue>,
        val answer: Map<String, String>, val phrases: Map<String, String>, val groups: List<Group>,
    )

    @Serializable
    private data class Reply(val conversation: Table? = null)

    private val core: (String) -> String? = { request -> if (coreIsAvailable) NativeClient.nativePlanView(request) else null }

    /** The core's bridge; a test can stand its own in, and [reset] puts the core back. */
    internal var bridge: (String) -> String? = core

    private val asked = mutableMapOf<String, Table>()
    private val json = Json { ignoreUnknownKeys = true }

    internal fun reset() {
        bridge = core
        synchronized(asked) { asked.clear() }
    }

    /** The table for [agent] ("Claude", "Codex"); null until the bridge answers. */
    fun table(agent: String = "Claude"): Table? = synchronized(asked) {
        asked[agent]?.let { return it }
        val answer = bridge("""{"schema":1,"conversation":{"style":"material","agent":"$agent"}}""") ?: return null
        val table = try {
            json.decodeFromString<Reply>(answer).conversation
        } catch (_: IllegalArgumentException) {
            null
        } ?: return null
        asked[agent] = table
        table
    }

    /** The core's answer for one item (`conversation_words::item`): [field] ("agent_type", "ms", "origin") goes in as [json] and comes back under its own name; null while the bridge can't answer. */
    internal fun item(field: String, json: String): String? {
        val answer = bridge("""{"schema":1,"item":{"$field":$json}}""") ?: return null
        return try {
            this.json.parseToJsonElement(answer).jsonObject["item"]?.jsonObject?.get(field)?.jsonPrimitive?.contentOrNull
        } catch (_: IllegalArgumentException) {
            null
        }
    }

    /** A phrase with its `{placeholders}` filled: `name_<by>` where the table has one, else `name`. */
    fun phrase(name: String, agent: String = "Claude", by: String? = null, fill: Map<String, String> = emptyMap()): String {
        val phrases = table(agent)?.phrases ?: return ""
        val words = by?.let { phrases["${name}_$it"] } ?: phrases[name] ?: return ""
        return fill.entries.fold(words) { line, (key, value) -> line.replace("{$key}", value) }
    }
}
