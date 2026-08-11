package com.farcooler.model

data class ToolRow(
    val id: String,
    val title: String,
    val kind: String,
    val status: ToolStatus,
    val locations: List<String>,
    val content: String? = null,
    val diff: Diff? = null,
)

/**
 * A subagent's dispatch row, and everything it did.
 *
 * The children are [TranscriptRow]s rather than a second row type, so a
 * subagent's messages, tools and gaps render through exactly the same views as
 * the top level. Nesting is where they live, not what they are.
 */
data class SubagentBlock(
    /** The `Task` call itself: title, status, location. */
    val tool: ToolRow,
    val children: List<TranscriptRow> = emptyList(),
    /** What it reported on finishing. Absent while it runs. */
    val summary: SubagentSummary? = null,
    /**
     * The turn ended before this reported back, so its outcome is unknown.
     * Distinct from failure, and emphatically distinct from success.
     */
    val interrupted: Boolean = false,
) {
    val id: String get() = tool.id

    /**
     * Still working, as far as anyone knows.
     *
     * Derived here rather than in each view because the interrupted case is
     * easy to miss and expensive to miss: interruption does NOT change the
     * tool's status — a cut-off subagent stays `InProgress` forever — so a
     * surface that asks the status alone keeps the block auto-expanded and
     * spinning for the rest of the session. The two Apple apps derived this
     * separately once and disagreed within a day.
     */
    val isRunning: Boolean
        get() = !interrupted &&
            (tool.status == ToolStatus.PENDING || tool.status == ToolStatus.IN_PROGRESS)

    /**
     * What a collapsed block says instead of its contents.
     *
     * Here rather than in each view for the same reason as [isRunning]: the two
     * apps wrote this twice and both said "1 tools".
     */
    val subtitle: String
        get() {
            // Ahead of the summary, deliberately. An interrupted block can
            // carry a partial one, and reporting a token count for a subagent
            // whose outcome nobody knows states the one thing this must never
            // say.
            if (interrupted) return "interrupted"
            val summary = summary ?: return if (isRunning) count(children.size, "step") else ""
            val tokens =
                if (summary.tokens >= 1000) "${summary.tokens / 1000}k tok"
                else "${summary.tokens} tok"
            // ROOT, not the device's locale: this is one line of a technical
            // subtitle that must read the same as the Mac's, and a phone set to
            // German would otherwise render "3,6s" beside a Mac's "3.6s" for
            // the same subagent.
            val seconds = String.format(java.util.Locale.ROOT, "%.1fs", summary.durationMs / 1000.0)
            return "${summary.agentType} · ${count(summary.toolUses.toInt(), "tool")} · $tokens · $seconds"
        }

    private fun count(n: Int, noun: String) = "$n $noun${if (n == 1) "" else "s"}"
}

/**
 * One row, with an identity of its own.
 *
 * The id is assigned when the row is created, NOT derived from its contents.
 * Deriving it meant asking the same question twice produced two rows with the
 * same id, and a keyed list over duplicate ids does not merely look odd — it
 * renders blank bands and repeats rows in the wrong places. Content is not
 * identity: two identical messages are two messages.
 */
data class TranscriptRow(val id: Int, val kind: Kind) {
    sealed interface Kind {
        /**
         * [parent] is carried even though nesting already places the row,
         * because coalescing needs it: an orphan whose block never arrived sits
         * at the top level beside the agent's own words, and merging the two
         * would re-create the very mis-attribution this fixes.
         */
        data class Message(val role: Role, val text: String, val parent: String?) : Kind

        data class Tool(val tool: ToolRow) : Kind

        data class Subagent(val block: SubagentBlock) : Kind

        data class Gap(val reason: GapReason) : Kind
    }
}

data class PendingPermission(
    val id: String,
    val toolCall: String,
    val options: List<PermissionOption>,
)

/**
 * Events in, something renderable out.
 *
 * Ported rather than reinvented, because a phone and a Mac that reduced the
 * same events differently would disagree about one session — the exact failure
 * the daemon-side derivation model exists to prevent. Every rule here, and
 * every reason for one, comes from `apps/shared/AgentKit/Sources/AgentKit/
 * Transcript.swift`; the unit tests are the Swift suite's, translated, so the
 * two stay honest about agreeing.
 *
 * Mutable rather than a value type, unlike the Swift original: Kotlin has no
 * `mutating` on a struct, and rebuilding an immutable transcript on every event
 * would copy every row several times a second during a streamed reply. The
 * class is confined to the main thread by [com.farcooler.net.AgentStream], and
 * [revision] is what tells Compose something changed.
 */
class Transcript {
    var rows: List<TranscriptRow> = emptyList()
        private set
    var plan: List<PlanEntry> = emptyList()
        private set
    var pendingPermission: PendingPermission? = null
        private set
    var agentMode: String? = null
        private set
    var availableModes: List<AgentChoice> = emptyList()
        private set
    var model: String? = null
        private set
    var availableModels: List<AgentChoice> = emptyList()
        private set

    /** Every selector the agent offers. Render one control each. */
    var configOptions: List<ConfigOption> = emptyList()
        private set

    /** What the agent calls this conversation, once it has named it. */
    var title: String? = null
        private set

    /**
     * Which protocol is carrying this conversation: `acp`, `claude`, or
     * `codex`. Defaults to `acp`, which is what a session that never said is.
     */
    var backend: String = "acp"
        private set

    /** Context-window usage, or null before the agent has reported any. */
    var contextUsed: Long? = null
        private set
    var contextSize: Long? = null
        private set

    /** How full the context window is, 0..1, or null if unknown. */
    val contextFraction: Double?
        get() {
            val used = contextUsed ?: return null
            val size = contextSize ?: return null
            if (size <= 0) return null
            return minOf(1.0, used.toDouble() / size.toDouble())
        }

    /**
     * The agent's slash commands, each with what it does.
     *
     * [AgentChoice], not names: the adapter has always sent a description and
     * the picker used to show only the name — a list of names is a list you
     * have to already know.
     */
    var availableCommands: List<AgentChoice> = emptyList()
        private set

    /** Written but not sent, in the order it will be sent. */
    var queue: List<QueuedPrompt> = emptyList()
        private set

    /** The seq to ask for on reconnect: one past the highest seen. */
    var cursor: Long = 0
        private set

    /**
     * Bumped on every change that a view could notice.
     *
     * The Swift original is a value type, so SwiftUI diffs it for free. A
     * Kotlin class has no such property, and a transcript is far too big to
     * compare field by field several times a second — so this is the one thing
     * Compose observes, and every mutation goes through [changed].
     */
    var revision: Long = 0
        private set

    /**
     * Whether the next message must start a new row rather than joining the
     * last one.
     *
     * Set at a turn boundary. Without it two consecutive turns' replies
     * coalesce — they are both agent and adjacent — and the transcript renders
     * "…prints hi to the console.Hello! I'm Claude Code…" as one paragraph,
     * with no seam where a whole turn began.
     */
    private var breakBeforeNextMessage = false

    /** The id the next row will take. Monotonic, never reused. */
    private var nextRowId = 0

    /**
     * One message drawn straight from the composer, and the row it drew.
     *
     * The row id is carried rather than the text alone so a withdrawal can
     * name its row exactly. See [unconfirmedEchoes].
     */
    private data class LocalEcho(val rowId: Int, val text: String)

    /**
     * Messages drawn straight from the composer that the daemon has not yet
     * accounted for.
     *
     * The composer used to PREDICT whether a message would be queued, by
     * reading fleet state while the thing it predicted read the agent
     * channel. In the window between a turn ending on one and the other
     * noticing, it guessed wrong: the echo was suppressed for a queue row
     * that never came, the CLI's own echo was dropped as a duplicate, and the
     * message reached the model without ever being drawn.
     *
     * So the client draws first and asks after. Entries are cleared at the
     * NEXT [AgentEvent.PromptQueue], which is the event that answers the
     * question — a queue that carries the text means it was held, and one that
     * does not means it went out. On the ordinary path that event never
     * arrives at all: `ChatSession::prompt` returns no events for a message
     * that goes straight to the backend, so an entry can sit here for the rest
     * of the epoch. That is not a leak to fix by dropping entries on a timer —
     * it is why the match in [AgentEvent.PromptQueue] CONSUMES. Each queue
     * item settles at most one echo, so an entry nobody ever answers can never
     * withdraw a row that a later, different message drew.
     *
     * Carrying the row id also removes a trap. Re-finding the row by text
     * would be correct only while no daemon-emitted `Message { role: User }`
     * can land after a live echo carrying the same words — which holds solely
     * because `send_next_queued` and `steer_queued` both emit their queue
     * event BEFORE the message. Swap those two lines in the daemon and a text
     * search would withdraw the genuine, already-delivered row instead of the
     * echo, and nothing on screen would look wrong. An id cannot make that
     * mistake, so the ordering stops being load-bearing.
     */
    private val unconfirmedEchoes = mutableListOf<LocalEcho>()

    private fun changed() {
        revision += 1
    }

    /**
     * Throw away everything and start again.
     *
     * Used when the daemon reports a different epoch: the cursor this held
     * counts positions in a stream that no longer exists, so the rows built
     * from it describe a conversation that is not the one being served. The
     * selectors survive, because they describe the AGENT rather than the stream
     * and re-arrive with the next `SessionStarted` anyway — clearing them would
     * blank the pickers on every toggle.
     */
    fun resetForNewEpoch() {
        rows = emptyList()
        plan = emptyList()
        queue = emptyList()
        pendingPermission = null
        cursor = 0
        nextRowId = 0
        breakBeforeNextMessage = false
        // An echo names an uncertain send from THIS epoch. Left alive, it can
        // collide with a message replayed into the new epoch — a real row —
        // and the next PromptQueue naming that text would delete restored
        // history that was never in question. Row ids restart here too, so a
        // surviving echo would not merely match by text: its recorded id would
        // name a real row of the new epoch.
        unconfirmedEchoes.clear()
        changed()
    }

    fun apply(events: List<Sequenced>) {
        for (item in events) {
            // Already folded in, so skipped rather than applied twice.
            //
            // Within one epoch the daemon numbers by position, so a seq below
            // the cursor names an event this transcript already holds. It can
            // still arrive: anything that re-delivers a batch — a reconnect, a
            // replay racing a push — hands back numbers already seen, and
            // applying them again renders the conversation twice.
            if (item.seq < cursor) continue
            cursor = item.seq + 1
            apply(item.event)
        }
        changed()
    }

    /**
     * Take the approval card down.
     *
     * The answer goes to the agent, which resumes without saying anything about
     * the request it was blocked on — no `Resolved` comes back — so a card that
     * waited for one stayed on screen after the work it was gating had already
     * happened.
     */
    fun clearPendingPermission() {
        pendingPermission = null
        changed()
    }

    /**
     * Show what the user just sent, before the agent says anything.
     *
     * The adapter echoes a `user_message_chunk` only when replaying a loaded
     * session, never during a live turn — so without this a message vanishes
     * the moment it is sent and does not reappear until the pane is reopened. A
     * chat that swallows your own words reads as broken even when the turn
     * underneath it is running perfectly.
     *
     * Called on EVERY send, including one written mid-turn. The client used to
     * decide up front which sends would be queued and skip drawing those; it
     * read that from stale state and a message that actually went straight out
     * was never drawn at all. So a mid-turn message is drawn here like any
     * other and withdrawn again by the [AgentEvent.PromptQueue] that reports it
     * held.
     */
    fun appendLocalUserMessage(text: String) {
        // Recorded before the append, because that is the id the row is about
        // to be given — the withdrawal below has to name this exact row.
        val rowId = nextRowId
        // Always the top level: what the user typed is addressed to the
        // session, never to one subagent inside it.
        append(TranscriptRow.Kind.Message(Role.USER, text, null))
        // The agent's reply is a new turn's worth of speech, not a continuation
        // of what the user just typed.
        breakBeforeNextMessage = true
        unconfirmedEchoes.add(LocalEcho(rowId, text))
        changed()
    }

    /**
     * Withdraw one row by the id it was created with.
     *
     * By id and not by contents: ids are handed out once per epoch and never
     * reused, so this withdraws the row that was drawn and nothing that merely
     * reads like it.
     */
    private fun removeRow(id: Int) {
        val index = rows.indexOfFirst { it.id == id }
        if (index < 0) return
        rows = rows.toMutableList().also { it.removeAt(index) }
    }

    /**
     * Show a selector's new value straight away.
     *
     * The adapter does not reliably send `config_option_update` after
     * `session/set_config_option` — it applies the change and says nothing — so
     * a picker that waited for confirmation snapped back to its old value and
     * read as broken. If a real update does arrive it overwrites this with the
     * same thing.
     */
    fun selectConfigOptionLocally(id: String, value: String) {
        setConfigValue(id, value)
        changed()
    }

    // MARK: - Placement

    /**
     * Which container a row belongs in.
     *
     * A parent nobody has seen resolves to the top level rather than being
     * dropped. The ring can trim a dispatch out from under its children, and a
     * reload can replay only part of a turn. Nothing is missing in that case
     * except the nesting, and a shorter transcript that looks complete is the
     * one failure this design refuses.
     */
    private sealed interface Destination {
        data object Top : Destination
        data class Block(val index: Int) : Destination
    }

    private fun destination(parent: String?): Destination {
        if (parent == null) return Destination.Top
        val index = rows.indexOfLast {
            (it.kind as? TranscriptRow.Kind.Subagent)?.block?.tool?.id == parent
        }
        return if (index < 0) Destination.Top else Destination.Block(index)
    }

    private fun append(kind: TranscriptRow.Kind, to: Destination = Destination.Top) {
        val row = TranscriptRow(nextRowId, kind)
        nextRowId += 1
        when (to) {
            Destination.Top -> rows = rows + row
            is Destination.Block -> {
                val block = (rows[to.index].kind as? TranscriptRow.Kind.Subagent)?.block ?: return
                rows = rows.replaced(
                    to.index,
                    rows[to.index].copy(
                        kind = TranscriptRow.Kind.Subagent(
                            block.copy(children = block.children + row)
                        )
                    ),
                )
            }
        }
    }

    /** The last row of whichever container this names. */
    private fun lastRow(destination: Destination): TranscriptRow? = when (destination) {
        Destination.Top -> rows.lastOrNull()
        is Destination.Block ->
            (rows[destination.index].kind as? TranscriptRow.Kind.Subagent)?.block?.children?.lastOrNull()
    }

    private fun replaceLastRow(destination: Destination, kind: TranscriptRow.Kind) {
        when (destination) {
            Destination.Top -> {
                if (rows.isEmpty()) return
                rows = rows.replaced(rows.lastIndex, rows.last().copy(kind = kind))
            }

            is Destination.Block -> {
                val block =
                    (rows[destination.index].kind as? TranscriptRow.Kind.Subagent)?.block ?: return
                if (block.children.isEmpty()) return
                val children = block.children.replaced(
                    block.children.lastIndex,
                    block.children.last().copy(kind = kind),
                )
                rows = rows.replaced(
                    destination.index,
                    rows[destination.index].copy(
                        kind = TranscriptRow.Kind.Subagent(block.copy(children = children))
                    ),
                )
            }
        }
    }

    /**
     * Update a call in place, wherever it lives.
     *
     * Three lookups, in order: the dispatch rows themselves, the top level,
     * then inside a block. The flat search this replaced found nothing for a
     * nested tool and fell through to appending, so one tool reporting progress
     * rendered as two rows — the real one inside the block and a half-built
     * duplicate beside it.
     */
    private fun applyToolUpdate(update: AgentEvent.ToolUpdate) {
        val dispatch = rows.indexOfLast {
            (it.kind as? TranscriptRow.Kind.Subagent)?.block?.tool?.id == update.id
        }
        if (dispatch >= 0) {
            val block = (rows[dispatch].kind as TranscriptRow.Kind.Subagent).block
            val merged = block.copy(
                tool = merged(block.tool, update),
                summary = update.subagent ?: block.summary,
            )
            rows = rows.replaced(dispatch, rows[dispatch].copy(kind = TranscriptRow.Kind.Subagent(merged)))
            return
        }

        val top = rows.indexOfLast {
            (it.kind as? TranscriptRow.Kind.Tool)?.tool?.id == update.id
        }
        if (top >= 0) {
            val tool = (rows[top].kind as TranscriptRow.Kind.Tool).tool
            rows = rows.replaced(top, rows[top].copy(kind = TranscriptRow.Kind.Tool(merged(tool, update))))
            return
        }

        for (index in rows.indices.reversed()) {
            val block = (rows[index].kind as? TranscriptRow.Kind.Subagent)?.block ?: continue
            val child = block.children.indexOfLast {
                (it.kind as? TranscriptRow.Kind.Tool)?.tool?.id == update.id
            }
            if (child < 0) continue
            val tool = (block.children[child].kind as TranscriptRow.Kind.Tool).tool
            val children = block.children.replaced(
                child,
                block.children[child].copy(kind = TranscriptRow.Kind.Tool(merged(tool, update))),
            )
            rows = rows.replaced(
                index,
                rows[index].copy(kind = TranscriptRow.Kind.Subagent(block.copy(children = children))),
            )
            return
        }

        // Nothing to update: an update whose call we never saw. Shown rather
        // than dropped, in whichever container it claims to belong to.
        append(
            TranscriptRow.Kind.Tool(
                ToolRow(
                    id = update.id,
                    title = update.title ?: update.id,
                    kind = "",
                    status = update.status,
                    locations = update.locations,
                    content = update.content,
                    diff = update.diff,
                )
            ),
            to = destination(update.parent),
        )
    }

    /**
     * The merge rules for a tool update, in one place so the lookup paths above
     * cannot drift apart from each other.
     */
    private fun merged(tool: ToolRow, update: AgentEvent.ToolUpdate) = tool.copy(
        status = update.status,
        // A call is renamed as it resolves — "Terminal" becomes the command,
        // "Read File" becomes the file — and the new name is the useful one.
        title = update.title?.takeIf { it.isNotEmpty() } ?: tool.title,
        content = update.content ?: tool.content,
        diff = update.diff ?: tool.diff,
        locations = update.locations.takeIf { it.isNotEmpty() } ?: tool.locations,
    )

    /**
     * Kept in the list rather than in a parallel map, so the control a user is
     * looking at and the value it shows can never be two different things.
     */
    private fun setConfigValue(id: String, value: String) {
        configOptions = configOptions.map { option ->
            if (option.id == id) option.copy(currentValue = value) else option
        }
        if (id == "model") model = value
        if (id == "mode") agentMode = value
    }

    private fun apply(event: AgentEvent) {
        when (event) {
            is AgentEvent.SessionStarted -> {
                backend = event.backend
                agentMode = event.agentMode
                availableModes = event.availableModes
                model = event.model
                if (event.availableModels.isNotEmpty()) availableModels = event.availableModels
                if (event.configOptions.isNotEmpty()) configOptions = event.configOptions
                // Only replace when the session actually offered some. A later
                // event carrying an empty list would otherwise empty the picker.
                if (event.availableCommands.isNotEmpty()) availableCommands = event.availableCommands
            }

            is AgentEvent.Message -> {
                val target = destination(event.parent)
                val last = lastRow(target)?.kind as? TranscriptRow.Kind.Message
                // Chunks of one message coalesce. One row per chunk would
                // render a streamed sentence as a column of one-word
                // paragraphs.
                //
                // Only within one container, and only across one parent:
                // merging past either boundary splices a subagent's sentence
                // onto the dispatching agent's and attributes it to the wrong
                // speaker.
                if (last != null &&
                    last.role == event.role &&
                    last.parent == event.parent &&
                    !(event.parent == null && breakBeforeNextMessage)
                ) {
                    replaceLastRow(
                        target,
                        TranscriptRow.Kind.Message(event.role, last.text + event.text, event.parent),
                    )
                } else {
                    append(
                        TranscriptRow.Kind.Message(event.role, event.text, event.parent),
                        to = target,
                    )
                }
                // The seam belongs to the top-level conversation; a subagent's
                // chunks must not consume it.
                if (event.parent == null) breakBeforeNextMessage = false
            }

            is AgentEvent.ToolCall -> {
                val tool = ToolRow(
                    id = event.id,
                    title = event.title,
                    kind = event.kind,
                    status = event.status,
                    locations = event.locations,
                )
                if (event.subagent) {
                    // A dispatch OWNS a block rather than being a row inside
                    // one, so it always lands at the level it was called from.
                    append(TranscriptRow.Kind.Subagent(SubagentBlock(tool = tool)))
                } else {
                    append(TranscriptRow.Kind.Tool(tool), to = destination(event.parent))
                }
            }

            is AgentEvent.ToolUpdate -> applyToolUpdate(event)

            // Wholesale, because the daemon sends the whole plan each time.
            is AgentEvent.Plan -> plan = event.entries

            is AgentEvent.Permission ->
                pendingPermission = PendingPermission(event.id, event.toolCall, event.options)

            is AgentEvent.Resolved ->
                if (pendingPermission?.id == event.id) pendingPermission = null

            // Resent every turn, so this arrives repeatedly with the same
            // contents. Replacing is right; appending would grow the picker
            // without bound.
            is AgentEvent.CommandsAvailable -> availableCommands = event.commands

            // Not a row. It is revised as the conversation goes on, and a line
            // of transcript per revision would bury the conversation it names.
            is AgentEvent.SessionInfo -> if (event.title.isNotEmpty()) title = event.title

            // Not a row either. It is resent constantly as a turn burns
            // context, and one line of transcript per report would bury the
            // conversation.
            is AgentEvent.Usage -> {
                contextUsed = event.used
                contextSize = event.size
            }

            is AgentEvent.ConfigSet -> setConfigValue(event.id, event.value)

            is AgentEvent.ModeSet -> agentMode = event.agentMode

            is AgentEvent.PromptQueue -> {
                // Wholesale, like the plan: the daemon sends the whole queue
                // on every change, and a client reconstructing it from adds
                // and removes could disagree with what will actually be
                // sent.
                queue = event.items
                // Anything drawn from the composer that turns out to be HELD
                // is withdrawn from the conversation — it has not joined it
                // yet, and in the queue it can still be edited or taken
                // back. Anything not named here went out, so it stays.
                // Either way every echo outstanding when this arrived is now
                // answered, so the list is cleared. An echo that no
                // PromptQueue ever answers — the whole ordinary path —
                // simply waits there; see [unconfirmedEchoes].
                //
                // Matched one at a time against a working copy, not with
                // `any`/`contains`: the same text sent twice before this
                // event leaves two identical echoes, and a non-consuming
                // match would let one held queue item cancel both rows —
                // deleting the one that genuinely went out, with no queue
                // entry left to show for it.
                //
                // The LAST matching echo, because when two identical sends
                // are outstanding it is the earlier one that went out and the
                // later one that is still waiting. Withdrawing the earlier
                // row would leave the transcript showing the agent's answer
                // above the question it answered.
                val unmatchedEchoes = unconfirmedEchoes.toMutableList()
                for (item in event.items) {
                    val index = unmatchedEchoes.indexOfLast { it.text == item.text }
                    if (index < 0) continue
                    val echo = unmatchedEchoes.removeAt(index)
                    // By id: the echo already knows which row it drew, so
                    // this cannot mistake a real user message for the echo of
                    // one.
                    removeRow(echo.rowId)
                }
                unconfirmedEchoes.clear()
            }

            is AgentEvent.TurnEnded -> {
                // Nothing to DRAW, but it is a seam: the next message begins a
                // new turn and must not be glued onto the tail of this one.
                breakBeforeNextMessage = true
                // A subagent still running when the turn ends never receives
                // its completion. Left alone it spins forever, and once the
                // view stops animating it reads as one that finished — a
                // subagent whose fate nobody knows wearing the mark of one that
                // reported back.
                rows = rows.map { row ->
                    val block = (row.kind as? TranscriptRow.Kind.Subagent)?.block ?: return@map row
                    if (block.tool.status != ToolStatus.PENDING &&
                        block.tool.status != ToolStatus.IN_PROGRESS
                    ) {
                        return@map row
                    }
                    row.copy(kind = TranscriptRow.Kind.Subagent(block.copy(interrupted = true)))
                }
            }

            // Never merged, never dropped. A gap that could be swallowed by a
            // neighbouring message would leave the user believing a transcript
            // is complete when it is not.
            is AgentEvent.Gap -> append(TranscriptRow.Kind.Gap(event.reason))
        }
    }
}

/** One element replaced, as a new list. */
private fun <T> List<T>.replaced(index: Int, value: T): List<T> {
    val out = toMutableList()
    out[index] = value
    return out
}
