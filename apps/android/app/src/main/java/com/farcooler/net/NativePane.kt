package com.farcooler.net

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.farcooler.model.AgentConversation
import com.farcooler.model.AgentRow
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject

/**
 * Where the conversation view's message goes (ov-374): `terminal.compose`,
 * which types it into claude's box on one line and presses Enter past the same
 * gate as `terminal tell`, or refuses with a word and types nothing. True when
 * claude was working and its own queue took it (R-29).
 */
fun interface ConversationSink {
    suspend fun compose(terminal: String, text: String): Boolean
}

/** The runner's `terminal.compose`, over this phone's client core. */
class CoreComposeSink(private val core: ClientCall) : ConversationSink {
    override suspend fun compose(terminal: String, text: String): Boolean {
        val answer = core.call(
            "terminal.compose",
            buildJsonObject {
                put("terminal", JsonPrimitive(terminal))
                put("text", JsonPrimitive(text))
            },
        )
        return (answer["queued"] as? JsonPrimitive)?.booleanOrNull ?: false
    }
}

/**
 * Where a held ask's answer goes (ov-370): the runner's `terminal.agent_answer`,
 * which writes it to the hook claude waits on. The first answer from any device
 * wins; a later one is refused `not_held`.
 */
fun interface AnswerSink {
    suspend fun answer(terminal: String, ask: String, option: String, answers: Map<String, String>)
}

/** The runner's `terminal.agent_answer`, over this phone's client core. */
class CoreAnswerSink(private val core: ClientCall) : AnswerSink {
    override suspend fun answer(terminal: String, ask: String, option: String, answers: Map<String, String>) {
        core.call(
            "terminal.agent_answer",
            buildJsonObject {
                put("terminal", JsonPrimitive(terminal))
                put("requestId", JsonPrimitive(ask))
                put("optionId", JsonPrimitive(option))
                if (answers.isNotEmpty()) put("answers", JsonObject(answers.mapValues { JsonPrimitive(it.value) }))
            },
        )
    }
}

/** Which view each pane remembers (R-27): the conversation, until it was switched to its terminal. */
interface PaneViewMemory {
    /** Whether [terminal] shows the conversation: yes until the pane was switched to its terminal. */
    fun wantsConversation(terminal: String): Boolean

    fun remember(terminal: String, conversation: Boolean)
}

/** A memory that lasts as long as the process, for tests and for a harness. */
class InMemoryPaneViews : PaneViewMemory {
    private val views = HashMap<String, Boolean>()
    override fun wantsConversation(terminal: String): Boolean = views[terminal] ?: true
    override fun remember(terminal: String, conversation: Boolean) {
        views[terminal] = conversation
    }
}

/**
 * Every conversation-view pane this connection has opened, by terminal id, so a
 * pane's draft and rows outlive the composables that show them: a tab swiped
 * away and evicted from the deck, a layout change, the pane switched to its
 * terminal and back. One per [Connection], so a pane on another runner gets a
 * model of its own.
 */
class NativePanes(
    private val scope: CoroutineScope,
    core: ClientCall,
    /** Where a pane's rows come from: the core's, unless a test stands in. */
    private val sourceFor: (String) -> AgentRowSource = { CoreRowSource(core, it) },
    private val sink: ConversationSink = CoreComposeSink(core),
    /** Where a held ask's answer goes (ov-370). */
    private val answers: AnswerSink? = CoreAnswerSink(core),
) {
    private val panes = HashMap<String, NativePaneModel>()

    /** The pane's model, made once and kept. */
    @Synchronized
    fun model(terminal: String, memory: PaneViewMemory): NativePaneModel = panes.getOrPut(terminal) {
        NativePaneModel(
            terminal = terminal,
            store = AgentRowStore(scope),
            source = sourceFor(terminal),
            sink = sink,
            memory = memory,
            scope = scope,
            answers = answers,
        )
    }
}

/**
 * One terminal-mode claude pane's conversation side (ov-374): its rows, the
 * composer's draft, and which of the two views shows. AgentKit-era iOS's
 * `NativePaneModel`, with its reviewed behavior.
 *
 * Nothing here touches the pane's process: the terminal under the conversation
 * is the same tmux pane, never respawned.
 *
 * Its state is Compose state, read where it's drawn, and written from the main
 * thread (the composables) or from [scope] (a send's outcome).
 */
class NativePaneModel(
    val terminal: String,
    val store: AgentRowStore,
    private val source: AgentRowSource,
    private val sink: ConversationSink,
    private val memory: PaneViewMemory,
    private val scope: CoroutineScope,
    /** Where a held ask's answer goes (ov-370); null where the runner takes none. */
    val answers: AnswerSink? = null,
) {
    /** The held ask whose answer is on its way, by its id. */
    var answering by mutableStateOf<String?>(null)
        private set

    /** Why an ask's answer didn't land, by the ask's id. */
    var answerIssues by mutableStateOf<Map<String, String>>(emptyMap())
        private set

    /**
     * Answer the held ask on [ask]'s row (ov-370, R-33): [option], and a
     * question's [given] answers. The runner writes it to claude's hook; nothing
     * is typed into its dialog. One at a time; refused, the row says why.
     */
    fun answer(ask: AgentRow.Ask, option: String, given: Map<String, String> = emptyMap()) {
        val sink = answers ?: return
        val id = ask.held ?: return
        if (answering != null || !AgentConversation.answerable(ask)) return
        answering = id
        answerIssues = answerIssues - id
        scope.launch {
            try {
                sink.answer(terminal, id, option, given)
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                val issue = when (val failure = AgentConversation.failure(e)) {
                    is AgentConversation.SendFailure.Refused -> AgentConversation.answerIssue(failure.what)
                    is AgentConversation.SendFailure.TimedOut -> AgentConversation.answerIssue(null, timedOut = true)
                    is AgentConversation.SendFailure.Lost ->
                        if (failure.notSent) AgentConversation.answerIssue(null) else AgentConversation.answerIssue(null, timedOut = true)
                }
                answerIssues = answerIssues + (id to issue)
            } finally {
                answering = null
            }
        }
    }

    /**
     * The composer's text, one line: line breaks become spaces as they arrive,
     * so what you see is what's sent. Return typed at the end sends, as the
     * keyboard's Send key says. Set through [onDraft].
     */
    var draft by mutableStateOf("")
        private set

    var sending by mutableStateOf(false)
        private set

    /** What stopped the last send, until the next one or a dismissal. */
    var issue by mutableStateOf<AgentConversation.SendIssue?>(null)

    /**
     * Messages claude's queue took that its transcript hasn't shown yet, drawn
     * as Queued rows below the list.
     */
    var queued by mutableStateOf<List<String>>(emptyList())
        private set

    /** How many messages this pane has sent, for the view to bring each one into view. */
    var sent by mutableIntStateOf(0)
        private set

    /** Whether the person wants the conversation here, remembered per pane (R-27). */
    var wantsConversation by mutableStateOf(memory.wantsConversation(terminal))
        private set

    /**
     * The runner stopped serving this pane's rows before any arrived: it shows
     * its terminal, with no switch, rather than an empty view.
     */
    var unavailable by mutableStateOf(false)
        private set

    /** What the pane shows: the conversation, when wanted and available. */
    val showing: Boolean get() = wantsConversation && !unavailable

    /** The follow loop was started, and not stopped since. */
    var following = false
        private set

    /** How many times the follow has been stopped, for the tests. */
    var stops = 0
        private set

    /** Whether this pane is the one on screen with the app in front. */
    private var onScreen = false

    fun switchTo(conversation: Boolean) {
        if (wantsConversation == conversation) return
        wantsConversation = conversation
        memory.remember(terminal, conversation)
        followIfDue()
    }

    /**
     * Whether the pane is on screen with the app in front. The follow runs only
     * then, and only while the conversation shows: a phone reaches its runner
     * over ssh, and a held follow per pane in the background is a held call per
     * pane nobody is reading.
     */
    fun setOnScreen(now: Boolean) {
        onScreen = now
        followIfDue()
    }

    private fun followIfDue() {
        val due = onScreen && showing
        // A loop that ended because the runner stopped serving rows isn't
        // following, whatever was set when it started.
        if (due && following && store.shown.value.phase == AgentRowStore.Phase.Unavailable) following = false
        if (due && !following) {
            following = true
            store.start(source)
        } else if (!due && following) {
            following = false
            stops += 1
            store.stop()
        }
    }

    /**
     * The pane's whole rule for the follow, in one place so the JVM tests hold it:
     * while the conversation is [offered] the follow is due as [live] says; when it
     * isn't (the setting went off, claude exited) the model lets go.
     */
    fun sync(offered: Boolean, live: Boolean) {
        if (offered) setOnScreen(live) else release()
    }

    /**
     * The pane left composition. The model outlives it, and a follow left running
     * would hold a call on the runner for a pane nobody has.
     */
    fun removed() = setOnScreen(false)

    /**
     * The conversation stopped being offered: the runner's setting turned off,
     * its build lost, claude exited. Stop following, and forget that the runner
     * said it had no rows, so the pane starts afresh when it's offered again. A
     * follow left running here, or ended Unavailable with [following] still set,
     * held the pane on "isn't being read" until a relaunch (ov-373 review 1).
     */
    fun release() {
        setOnScreen(false)
        if (unavailable) unavailable = false
    }

    /**
     * The runner said it doesn't serve this pane's rows. With none held, the
     * pane falls back to its terminal; with some, the view says they're stale
     * over them.
     */
    fun phaseChanged() {
        val shown = store.shown.value
        // Only the loop's own word counts: a phase left behind by an earlier
        // follow says nothing about a pane that isn't being followed now.
        if (following && shown.phase == AgentRowStore.Phase.Unavailable && shown.rows.isEmpty() && !unavailable) {
            unavailable = true
            followIfDue()
        }
    }

    val canSend: Boolean
        get() {
            val text = draft.trim()
            return !sending && text.isNotEmpty() && text.length <= AgentConversation.LONGEST && !store.shown.value.isStale
        }

    /** The text field's change: one line, and a trailing Return sends. */
    fun onDraft(text: String) {
        if (text.endsWith("\n") && text.dropLast(1) == draft) {
            send()
            return
        }
        draft = AgentConversation.flattened(text)
    }

    /** Send the draft. The outcome is kept here, whether or not a view is there to see it. */
    fun send() {
        val text = draft.trim()
        if (!canSend) {
            if (text.length > AgentConversation.LONGEST) issue = AgentConversation.SendIssue.Said(AgentConversation.TOO_LONG)
            return
        }
        if (AgentConversation.isCommand(text)) {
            issue = AgentConversation.SendIssue.Said(AgentConversation.COMMAND)
            return
        }
        sending = true
        issue = null
        scope.launch {
            try {
                val wasQueued = sink.compose(terminal, text)
                if (draft.trim() == text) draft = ""
                if (wasQueued) queued = queued + text
                sent += 1
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                issue = AgentConversation.issue(AgentConversation.failure(e))
            } finally {
                sending = false
            }
        }
    }

    /** The page above the oldest row held. */
    fun loadOlder() = store.loadOlder(source)

    /** Drop a local Queued echo once the transcript shows the message. */
    fun settleQueued() {
        if (queued.isEmpty()) return
        val newest = store.shown.value.rows.takeLast(40)
        val left = AgentConversation.unsettled(queued, newest)
        if (left != queued) queued = left
    }
}
