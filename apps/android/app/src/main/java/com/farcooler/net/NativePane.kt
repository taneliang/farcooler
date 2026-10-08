package com.farcooler.net

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.farcooler.model.AgentConversation
import com.farcooler.model.AgentRow
import com.farcooler.model.BringHere
import com.farcooler.model.OutgoingImage
import java.util.Base64
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject

/**
 * Where the conversation view's message goes (ov-374): `terminal.compose`, which
 * types it into claude's box and presses Enter past the same gate as `terminal
 * tell`, or refuses with a word and types nothing. With the runner's `compose`
 * (ov-367), its line breaks, images and slash command too; the client core
 * stages the images first where the runner takes that (ov-393). True when claude
 * was working and its own queue took it (R-29).
 */
fun interface ConversationSink {
    suspend fun compose(terminal: String, text: String, images: List<OutgoingImage>): Boolean
}

/** The runner's `terminal.compose`, over this phone's client core. */
class CoreComposeSink(private val core: ClientCall) : ConversationSink {
    override suspend fun compose(terminal: String, text: String, images: List<OutgoingImage>): Boolean {
        val answer = core.call(
            "terminal.compose",
            buildJsonObject {
                put("terminal", JsonPrimitive(terminal))
                put("text", JsonPrimitive(text))
                // The core stages each image on a runner that takes uploads
                // (`compose_upload`), in chunks, and carries a small one inside the
                // compose on one that doesn't; either way it's handed the bytes here.
                if (images.isNotEmpty()) {
                    put(
                        "images",
                        buildJsonArray {
                            for (image in images) {
                                add(
                                    buildJsonObject {
                                        put("mime", JsonPrimitive(image.mime))
                                        put("base64", JsonPrimitive(Base64.getEncoder().encodeToString(image.data)))
                                    },
                                )
                            }
                        },
                    )
                }
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

/**
 * Stop and Send now (ov-368): the runner presses one Esc, or claude's ctrl+x
 * ctrl+s, in the pane's TUI, past the same gate as a send, and answers once claude
 * took it; or it refuses with a word and presses nothing.
 */
interface InterruptSink {
    suspend fun interrupt(terminal: String)

    suspend fun sendNow(terminal: String)
}

/** The runner's `terminal.interrupt` and `terminal.send_now`, over this phone's client core. */
class CoreInterruptSink(private val core: ClientCall) : InterruptSink {
    override suspend fun interrupt(terminal: String) {
        core.call("terminal.interrupt", buildJsonObject { put("terminal", JsonPrimitive(terminal)) })
    }

    override suspend fun sendNow(terminal: String) {
        core.call("terminal.send_now", buildJsonObject { put("terminal", JsonPrimitive(terminal)) })
    }
}

/**
 * Bring here (ov-369, R-28): the runner's `terminal.bring_draft` reads claude's
 * box with nothing typed, or with [expected], clears it of exactly that.
 */
fun interface DraftSink {
    suspend fun bringDraft(terminal: String, expected: String?): Pair<String, Boolean>
}

/** The runner's `terminal.bring_draft`, over this phone's client core. */
class CoreDraftSink(private val core: ClientCall) : DraftSink {
    override suspend fun bringDraft(terminal: String, expected: String?): Pair<String, Boolean> {
        val answer = core.call(
            "terminal.bring_draft",
            buildJsonObject {
                put("terminal", JsonPrimitive(terminal))
                if (expected != null) put("expected", JsonPrimitive(expected))
            },
        )
        val text = (answer["text"] as? JsonPrimitive)?.contentOrNull ?: ""
        return text to ((answer["cleared"] as? JsonPrimitive)?.booleanOrNull ?: false)
    }
}

/** Which view each pane remembers (R-27): the conversation, until it was switched to its terminal. */
interface PaneViewMemory {
    /** Whether [terminal] shows the conversation: yes until the pane was switched to its terminal. */
    fun wantsConversation(terminal: String): Boolean

    fun remember(terminal: String, conversation: Boolean)

    /** The composer's text saved for [terminal] (ov-369 F4, R-38), or "" for none. */
    fun draft(terminal: String): String = ""

    /** Save [text] as [terminal]'s composer draft; "" removes it. Text only, already capped by [NativeDraft.capped]. */
    fun saveDraft(terminal: String, text: String) {}
}

/** The composer's saved draft (ov-369 F4, R-38): at most 64 KB of UTF-8, written after a quiet 300 ms. */
object NativeDraft {
    const val CAP_BYTES = 64 * 1024
    const val DELAY_MS = 300L

    /** [text] cut to at most [CAP_BYTES] of UTF-8, never inside a character. */
    fun capped(text: String): String {
        if (text.toByteArray(Charsets.UTF_8).size <= CAP_BYTES) return text
        var used = 0
        var end = 0
        while (end < text.length) {
            val point = text.codePointAt(end)
            val size = String(Character.toChars(point)).toByteArray(Charsets.UTF_8).size
            if (used + size > CAP_BYTES) break
            used += size
            end += Character.charCount(point)
        }
        return text.substring(0, end)
    }
}

/** A memory that lasts as long as the process, for tests and for a harness. */
class InMemoryPaneViews : PaneViewMemory {
    private val views = HashMap<String, Boolean>()
    override fun wantsConversation(terminal: String): Boolean = views[terminal] ?: true
    override fun remember(terminal: String, conversation: Boolean) {
        views[terminal] = conversation
    }

    private val drafts = HashMap<String, String>()
    override fun draft(terminal: String): String = drafts[terminal] ?: ""
    override fun saveDraft(terminal: String, text: String) {
        if (text.isEmpty()) drafts.remove(terminal) else drafts[terminal] = text
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
    private val interruptSink: InterruptSink = CoreInterruptSink(core),
    /** Where Bring here reads and clears claude's box (ov-369). */
    private val draftSink: DraftSink = CoreDraftSink(core),
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
            interruptSink = interruptSink,
            draftSink = draftSink,
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
    /** Where Stop and Send now go, handed to [keys] where the runner offers them. */
    private val interruptSink: InterruptSink? = null,
    /** Where Bring here goes, handed to [drafts] where the runner offers it (ov-369). */
    private val draftSink: DraftSink? = null,
    /** How long the draft is quiet before it is saved (ov-369 F4); tests shorten it. */
    private val draftDelayMs: Long = NativeDraft.DELAY_MS,
) {
    /** The agent the pane runs, as its preset says: `claude` or `codex` (ov-416). */
    var preset by mutableStateOf("claude")

    /** The agent's name, as the conversation's words say it. */
    val agent: String get() = AgentConversation.agentName(preset)

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
                    is AgentConversation.SendFailure.Refused -> AgentConversation.answerIssue(failure.what, agent = agent)
                    is AgentConversation.SendFailure.TimedOut -> AgentConversation.answerIssue(null, timedOut = true, agent = agent)
                    is AgentConversation.SendFailure.Lost ->
                        if (failure.notSent) AgentConversation.answerIssue(null, agent = agent)
                        else AgentConversation.answerIssue(null, timedOut = true, agent = agent)
                }
                answerIssues = answerIssues + (id to issue)
            } finally {
                answering = null
            }
        }
    }

    /**
     * The composer's text. Against a runner without `compose`, one line: line
     * breaks become spaces as they arrive, so what you see is what's sent, and a
     * Return typed at the end sends, as the keyboard's Send key says. With it, as
     * typed: Return is a new line, and Send sends. Set through [onDraft].
     */
    var draft by mutableStateOf(memory.draft(terminal))
        private set

    private var savingDraft: kotlinx.coroutines.Job? = null

    /** Every change to [draft] goes through here, so it is saved: after [draftDelayMs] quiet, or at once when empty (a confirmed send must not be undone by a late write). */
    private fun changeDraft(text: String) {
        draft = text
        savingDraft?.cancel()
        savingDraft = null
        if (text.isEmpty()) {
            memory.saveDraft(terminal, "")
            return
        }
        savingDraft = scope.launch {
            kotlinx.coroutines.delay(draftDelayMs)
            memory.saveDraft(terminal, NativeDraft.capped(text))
        }
    }

    /** The draft's pending save, finished. For tests. */
    suspend fun draftSaved() {
        savingDraft?.join()
    }

    /**
     * Whether the runner takes line breaks, images and slash commands (`compose`,
     * ov-367), as its hello said. Without it, one line.
     */
    var rich by mutableStateOf(false)
        private set

    /** Images to send with the text, in order (ov-404). Only with [rich]. */
    var images by mutableStateOf<List<OutgoingImage>>(emptyList())
        private set

    /** Where Stop and Send now go (ov-368): the runner's connection where it serves `terminal_interrupt`; null, and neither is offered. */
    var keys by mutableStateOf<InterruptSink?>(null)
        private set

    /** A Stop or a Send now on its way, until the runner answers. */
    var pressing by mutableStateOf<AgentConversation.PaneKey?>(null)
        private set

    /** Where Bring here reads and clears claude's box (ov-369): the runner's connection where it serves `bring_draft`; null, and Show terminal alone. */
    var drafts by mutableStateOf<DraftSink?>(null)
        private set

    /** A Bring here on its way, until the runner answers the clear. */
    var bringing by mutableStateOf(false)
        private set

    private val nextImage = java.util.concurrent.atomic.AtomicInteger()

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
        if (following && shown.phase == AgentRowStore.Phase.Unavailable && shown.rows.none { it.id != AgentRow.HINT_ID } && !unavailable) {
            unavailable = true
            followIfDue()
        }
    }

    /**
     * What the runner offers this pane, from its hello's capabilities: line
     * breaks, images and commands with `compose` (ov-367), Stop and Send now with
     * `terminal_interrupt` (ov-368).
     */
    fun offer(rich: Boolean, interrupts: Boolean, bring: Boolean = false) {
        // Bring here with `bring_draft` (ov-369).
        val drafting = if (bring) draftSink else null
        if (drafts !== drafting) drafts = drafting
        if (this.rich != rich) {
            this.rich = rich
            if (!rich) {
                images = emptyList()
                // The draft was kept as typed, or flattened as it came in.
                changeDraft(AgentConversation.flattened(draft))
            }
        }
        val sink = if (interrupts) interruptSink else null
        if (keys !== sink) keys = sink
    }

    /** The longest message the box takes now. */
    val longestNow: Int get() = AgentConversation.longest(rich)

    /**
     * The draft as it's sent: trimmed of spaces at the ends on one line; with
     * `compose`, as typed, the runner trimming the ends but keeping an indent.
     */
    private val outgoing: String get() = if (rich) draft else draft.trim(' ', '\t')

    private val hasText: Boolean get() = draft.isNotBlank()

    val canSend: Boolean
        get() = !sending && (hasText || images.isNotEmpty()) && outgoing.length <= longestNow && !store.shown.value.isStale

    /**
     * The prompt claude's own box suggests, offered as the composer's placeholder
     * while nothing is typed ([AgentConversation.suggestion], ov-409).
     */
    val suggestion: String?
        get() = AgentConversation.suggestion(
            AgentConversation.newestTurn(store.shown.value.rows), draft, store.shown.value.isStale,
        )

    /** claude's generic `Try "…"` example, in place of "Message Claude" while nothing is typed. A hint: a tap and Tab do not take it. */
    val hint: String?
        get() = AgentConversation.hint(store.shown.value.rows, draft, store.shown.value.isStale)

    /** A tap on the suggestion, or Tab from a hardware keyboard: it becomes the draft, to edit. Never sent. True when there was one. */
    fun takeSuggestion(): Boolean {
        val words = suggestion ?: return false
        onDraft(words)
        return true
    }

    /** The text field's change: as typed with `compose`; one line, and a trailing Return sends, without. */
    fun onDraft(text: String) {
        if (rich) {
            changeDraft(text)
            return
        }
        if (text.endsWith("\n") && text.dropLast(1) == draft) {
            send()
            return
        }
        changeDraft(AgentConversation.flattened(text))
    }

    /**
     * The one path a photo takes into the composer, from the picker, a paste or a
     * test standing in for either: [datas] read (null where one didn't load, as a
     * photo in the cloud and not on the device doesn't), converted by [convert]
     * where the runner wouldn't take it as it is, off the main thread, and added;
     * or said why not.
     */
    suspend fun attachPicked(datas: List<ByteArray?>, convert: (ByteArray) -> OutgoingImage.Converted?) {
        if (!rich) return
        val made = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default) {
            datas.map { data -> data?.let { OutgoingImage.make(nextImageId(), it, convert) } }
        }
        attach(made.filterNotNull())
        if (made.any { it == null }) issue = AgentConversation.SendIssue.Said(AgentConversation.UNREADABLE_IMAGE)
    }

    /** Add [new] after the images already waiting, up to the most a message takes. */
    fun attach(new: List<OutgoingImage>) {
        if (!rich) return
        val room = maxOf(0, AgentConversation.MOST_IMAGES - images.size)
        images = images + new.take(room)
        if (new.size > room) issue = AgentConversation.SendIssue.Said(AgentConversation.TOO_MANY_IMAGES)
    }

    /** A new image's id, for the loader to make one with. */
    fun nextImageId(): Int = nextImage.getAndIncrement()

    /** Take the image [id] out of the message. */
    fun detach(id: Int) {
        images = images.filter { it.id != id }
    }

    /** How many more images the message takes. */
    val imageRoom: Int get() = maxOf(0, AgentConversation.MOST_IMAGES - images.size)

    /** Send the draft and its images. The outcome is kept here, whether or not a view is there to see it. */
    fun send() {
        val text = outgoing
        val going = images
        if (!canSend) {
            if (outgoing.length > longestNow) issue = AgentConversation.SendIssue.Said(AgentConversation.tooLong(rich))
            return
        }
        // Without `compose`, a slash or a bang would open claude's command picker
        // or its shell, which Enter would then run. With it, the runner drives the
        // picker, and refuses what it can't.
        if (!rich && AgentConversation.isCommand(text)) {
            issue = AgentConversation.SendIssue.Said(AgentConversation.COMMAND)
            return
        }
        sending = true
        issue = null
        scope.launch {
            try {
                val wasQueued = sink.compose(terminal, text, going)
                if (outgoing == text) changeDraft("")
                detachAll(going)
                if (wasQueued) queued = queued + AgentConversation.echo(text, going.size)
                sent += 1
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                issue = AgentConversation.issue(AgentConversation.failure(e), command = text.trim().startsWith("/"), agent = agent)
            } finally {
                sending = false
            }
        }
    }

    private fun detachAll(sent: List<OutgoingImage>) {
        val ids = sent.map { it.id }.toSet()
        images = images.filter { it.id !in ids }
    }

    // Stop and Send now.

    /** Whether claude is working on a turn, as the newest turn's row says (not while a dialog is up). */
    val working: Boolean get() = AgentConversation.isWorking(AgentConversation.newestTurn(store.shown.value.rows))

    /** Whether Stop is offered: the runner serves it and claude is working. */
    val offersStop: Boolean get() = keys != null && AgentConversation.pressesKeys(preset) && working && !store.shown.value.isStale

    /** Whether Send now is offered on a Queued row. */
    val offersSendNow: Boolean get() = offersStop

    /** Stop the turn. */
    fun stop() = press(AgentConversation.PaneKey.Stop)

    /** Send what waits in claude's queue now: a Queued row's Send now. */
    fun sendNow() = press(AgentConversation.PaneKey.SendNow)

    private fun press(key: AgentConversation.PaneKey) {
        val keys = keys ?: return
        if (pressing != null || !offersStop) return
        pressing = key
        issue = null
        scope.launch {
            try {
                when (key) {
                    AgentConversation.PaneKey.Stop -> keys.interrupt(terminal)
                    AgentConversation.PaneKey.SendNow -> keys.sendNow(terminal)
                }
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                issue = AgentConversation.keyIssue(AgentConversation.failure(e), key)
            } finally {
                pressing = null
            }
        }
    }

    // Bring here (ov-369, R-28).

    /** Whether the draft line offers Bring here: claude, on a runner that serves it. */
    val offersBringHere: Boolean get() = drafts != null && preset.startsWith("claude")

    /** Move the box's draft into the composer, ahead of what it holds, and clear the box. The draft line's Bring here. */
    fun bringHere() {
        val drafts = drafts ?: return
        if (bringing || !offersBringHere) return
        bringing = true
        issue = null
        scope.launch {
            try {
                issue = BringHere.run(
                    agent = agent,
                    read = { answered { drafts.bringDraft(terminal, null).first } },
                    place = { text -> changeDraft(BringHere.merged(text, draft)) },
                    withdraw = { text -> changeDraft(BringHere.withdrawn(text, draft)) },
                    clear = { text -> answered { drafts.bringDraft(terminal, text).second } },
                )
            } finally {
                bringing = false
            }
        }
    }

    private suspend fun <T> answered(call: suspend () -> T): BringHere.Answer<T> = try {
        BringHere.Answer.Took(call())
    } catch (e: kotlinx.coroutines.CancellationException) {
        throw e
    } catch (e: Exception) {
        BringHere.Answer.Failed(AgentConversation.failure(e))
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
