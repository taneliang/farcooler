package com.farcooler.net

import android.util.Base64
import com.farcooler.core.ClientCore
import com.farcooler.model.AgentEvent
import com.farcooler.model.Sequenced
import com.farcooler.model.Transcript
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject

/**
 * One terminal's agent session, live, over the same host connection a
 * terminal's screen already polls through.
 *
 * Holds a [Transcript] and nothing else derived: the agent mode, the available
 * modes and the commands all arrive on the transcript itself, from the events
 * the daemon sent — never recomputed here, for the same reason [Connection]
 * never computes a workspace's state.
 */
class AgentStream(
    private val terminal: String,
    private val core: ClientCore,
    private val scope: CoroutineScope,
) {
    val transcript = Transcript()

    /**
     * Bumped whenever [transcript] changes, because a class is not a value and
     * Compose cannot diff one. Reading this in a composable is what makes the
     * conversation redraw.
     */
    private val _revision = MutableStateFlow(0L)
    val revision: StateFlow<Long> = _revision.asStateFlow()

    private val _connectionError = MutableStateFlow<String?>(null)
    val connectionError: StateFlow<String?> = _connectionError.asStateFlow()

    private var pollTask: Job? = null

    /** The run of the stream this transcript was built from. */
    private var epoch: Long = 0

    private val json = Json { ignoreUnknownKeys = true }

    fun start() {
        pollTask?.cancel()
        pollTask = scope.launch {
            while (isActive) {
                pump()
                // Not the Mac's 200 ms: that poll is a local call into a daemon
                // on the same machine, and this one is an SSH round trip. A chat
                // transcript has no per-frame redraw to protect the way a
                // terminal's screen does, so a slower, still-brisk cadence costs
                // far less battery for a difference nobody reading a
                // conversation would notice.
                delay(POLL_INTERVAL_MS)
            }
        }
    }

    fun stop() {
        pollTask?.cancel()
        pollTask = null
    }

    /**
     * Ask for everything after what we already hold.
     *
     * The cursor comes from the transcript rather than a counter kept here, so
     * a reconnect — or this object simply being recreated when the tab strip
     * switches to a different agent pane — cannot skip or repeat events after a
     * gap.
     */
    private suspend fun pump() {
        try {
            val data = core.call(
                "terminal.agent_subscribe",
                Connection.args(
                    "terminal" to terminal,
                    "fromSeq" to transcript.cursor,
                    "epoch" to epoch,
                ),
            )
            val batch = json.decodeFromJsonElement(Batch.serializer(), data)

            // Epoch 0 with nothing in it means the daemon has no agent session
            // for this terminal at all — `AgentSupervisor::replay` returns
            // exactly that for a terminal it has never seen. A client starting
            // at epoch 0 matches it, finds no events, and returns, which looked
            // from the outside like a conversation nobody had started yet. It is
            // not: it is a pane that has no shim behind it, and the two want
            // different words.
            if (batch.epoch == 0L && batch.events.isEmpty() && transcript.rows.isEmpty()) {
                _connectionError.value =
                    "No agent session on pane ${terminal.take(8)} yet. It may still be starting."
                return
            }

            // A different epoch means the stream restarted — the pane was
            // toggled, or the shim came back — and every number this holds
            // counts positions in a conversation that no longer exists. The
            // batch that comes back is the whole transcript, so it replaces
            // rather than appends.
            if (batch.epoch != epoch) {
                epoch = batch.epoch
                transcript.resetForNewEpoch()
            } else if (batch.events.isEmpty()) {
                _connectionError.value = null
                return
            }

            val decoded = batch.events.map { frame ->
                // A malformed single frame does not fail the whole batch —
                // `AgentEvent.decode` already turns an event this client does
                // not recognise into a gap rather than throwing.
                Sequenced(frame.seq, AgentEvent.decode(frame.payloadJson))
            }
            transcript.apply(decoded)
            _connectionError.value = null
        } catch (e: Exception) {
            // Leaving this pane cancels the poll. That is not something to tell
            // the reader about, and the banner would outlive the screen it was
            // about.
            e.rethrowIfCancellation()
            _connectionError.value = e.message ?: "The host stopped answering."
        } finally {
            _revision.value = transcript.revision
        }
    }

    fun send(text: String, images: List<Attachment> = emptyList()) {
        scope.launch {
            // Drawn immediately, whether or not this turns out to be queued —
            // the daemon's next PromptQueue withdraws it if it was held.
            // Predicting the outcome here read fleet state about a decision
            // taken on the agent channel, and the two could disagree for the
            // few milliseconds between a turn ending and this client
            // noticing: the echo was suppressed for a queue row that never
            // came, the CLI's own echo was dropped as a duplicate, and the
            // message reached the model without ever being drawn.
            transcript.appendLocalUserMessage(text)
            _revision.value = transcript.revision
            // Base64 through the core, which decodes it into the protocol's
            // bytes. The picture travels WITH the prompt; there is no path,
            // because a path from a phone means nothing on the host.
            val args = buildJsonObject {
                put("terminal", JsonPrimitive(terminal))
                put("text", JsonPrimitive(text))
                if (images.isNotEmpty()) {
                    put(
                        "images",
                        JsonArray(
                            images.map { attachment ->
                                buildJsonObject {
                                    put("mime", JsonPrimitive(attachment.mime))
                                    put(
                                        "base64",
                                        JsonPrimitive(
                                            Base64.encodeToString(attachment.data, Base64.NO_WRAP)
                                        ),
                                    )
                                }
                            }
                        ),
                    )
                }
            }
            attempt { core.call("terminal.agent_prompt", args) }
        }
    }

    /** Rewrite a message that has not gone out yet. */
    fun editQueued(id: String, text: String) = fireAndForget(
        "terminal.agent_edit_queued",
        Connection.args("terminal" to terminal, "queuedId" to id, "text" to text),
    )

    /** Send a queued message into the turn already running. */
    fun steerQueued(id: String) = fireAndForget(
        "terminal.agent_steer_queued",
        Connection.args("terminal" to terminal, "queuedId" to id),
    )

    /** Take back a message that has not gone out yet. */
    fun cancelQueued(id: String) = fireAndForget(
        "terminal.agent_cancel_queued",
        Connection.args("terminal" to terminal, "queuedId" to id),
    )

    fun setConfig(id: String, value: String) {
        // Shown before it is confirmed. The adapter applies
        // `session/set_config_option` and says nothing back, so a picker that
        // waited for an echo snapped to its old value and read as a control
        // that does nothing.
        transcript.selectConfigOptionLocally(id, value)
        _revision.value = transcript.revision
        fireAndForget(
            "terminal.agent_set_config",
            Connection.args("terminal" to terminal, "configId" to id, "value" to value),
        )
    }

    fun answer(requestId: String, optionId: String) {
        // Taken down on tap, not on an echo. The agent resumes without
        // acknowledging the request it was blocked on, so a card that waited for
        // confirmation sat there after the work it gated had happened.
        transcript.clearPendingPermission()
        _revision.value = transcript.revision
        fireAndForget(
            "terminal.agent_answer",
            Connection.args("terminal" to terminal, "requestId" to requestId, "optionId" to optionId),
        )
    }

    fun setMode(mode: String) = fireAndForget(
        "terminal.agent_set_mode",
        Connection.args("terminal" to terminal, "mode" to mode),
    )

    /**
     * Stop the turn that is running.
     *
     * The Mac has had this on its composer since the native agent view landed;
     * neither phone app ever wired it up, so an agent that had gone off in the
     * wrong direction could only be stopped by switching the pane back to a
     * terminal and pressing Ctrl-C — which is the one thing a chat surface is
     * supposed to make unnecessary.
     */
    fun cancel() = fireAndForget(
        "terminal.agent_cancel",
        Connection.args("terminal" to terminal),
    )

    private fun fireAndForget(method: String, args: JsonObject) {
        scope.launch { attempt { core.call(method, args) } }
    }

    /** An image on its way to the agent, with the type the host will be told. */
    data class Attachment(val mime: String, val data: ByteArray) {
        override fun equals(other: Any?) =
            other is Attachment && mime == other.mime && data.contentEquals(other.data)

        override fun hashCode() = 31 * mime.hashCode() + data.contentHashCode()
    }

    @Serializable
    private data class EventFrame(val seq: Long, val payloadJson: String)

    @Serializable
    private data class Batch(
        val events: List<EventFrame> = emptyList(),
        /** Which run of the stream these numbers belong to. */
        val epoch: Long = 0,
    )

    private companion object {
        const val POLL_INTERVAL_MS = 700L
    }
}
