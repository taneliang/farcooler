package com.farcooler.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.AddPhotoAlternate
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.outlined.ChatBubbleOutline
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.model.AgentActivity
import com.farcooler.model.AgentChoice
import com.farcooler.model.ComposerToken
import com.farcooler.model.ConfigOption
import com.farcooler.model.QueuedPrompt
import com.farcooler.model.activeToken
import com.farcooler.net.AgentPhase
import com.farcooler.net.AgentStream
import com.farcooler.net.Connection
import com.farcooler.net.TerminalRef
import com.farcooler.net.Waited
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive

/**
 * One agent session, as a conversation.
 *
 * The surface [TerminalPane] swaps in when the daemon reports the pane is
 * hosting a chat. Unlike the Mac there is no tmux rectangle to draw into: this
 * pane is always the whole screen, and the tab strip that lets you leave it
 * lives below every pane rather than below this one — [WorkspaceScreen] owns
 * that, not this.
 *
 * ## The stream outlives the tab tap now
 *
 * This built a fresh [AgentStream] with `remember(ref.terminalId)`, because one
 * copy of this screen was re-pointed at whichever pane the strip had selected.
 * So leaving a conversation and coming back threw away the transcript, its
 * scroll position and the poll, and rebuilt all three from `fromSeq = 0`. Half
 * of F-3 in the parity inventory; the other half was the terminal.
 *
 * A pane is mounted for as long as it is a tab now, so the key is gone — there
 * is nothing left for it to guard against, and anything in one would be a way
 * for this stream to be rebuilt without the pane being. What starts and stops
 * the POLL is [live], which is not the same question: a tab nobody is reading,
 * and every tab while the app is backgrounded, keeps its transcript and spends
 * nothing on the link.
 */
@Composable
fun AgentScreen(
    model: AppModel,
    ref: TerminalRef,
    connection: Connection,
    /** Whether this pane is the one being read. See [TerminalPane]. */
    live: Boolean = true,
) {
    val scope = rememberCoroutineScope()
    val stream = remember { AgentStream(ref.terminalId, connection.core, scope) }
    // Resumed rather than restarted, and [AgentStream.pump] is what makes that
    // free: it asks from `transcript.cursor`, so a poll that stopped for ten
    // minutes comes back asking for exactly what it missed rather than for the
    // whole conversation again.
    LaunchedEffect(live) {
        if (live) stream.start() else stream.stop()
    }
    DisposableEffect(stream) {
        onDispose { stream.stop() }
    }

    // A class is not a value, so this is what makes the conversation redraw.
    val revision by stream.revision.collectAsStateWithLifecycle()
    val phase by stream.phase.collectAsStateWithLifecycle()
    val transcript = stream.transcript

    val terminal = model.fleet.terminal(ref)
    val isWorking = terminal?.agent == AgentActivity.WORKING
    val harness = terminal?.preset?.takeIf { it.isNotEmpty() }
        ?.replaceFirstChar { it.uppercase() } ?: "the agent"

    val listState = rememberLazyListState()
    var followingTail by remember { mutableStateOf(true) }

    // Whether the reader is parked at the tail.
    LaunchedEffect(listState) {
        snapshotFlow {
            val last = listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0
            last >= listState.layoutInfo.totalItemsCount - 2
        }.collect { followingTail = it }
    }

    // Keyed on the REVISION, not the row count. A streamed reply coalesces into
    // the row already on screen, so the count does not change while the text
    // grows off the bottom. And only while the reader is at the tail: scrolling
    // to the end on every event made reading anything older impossible.
    LaunchedEffect(revision) {
        if (!followingTail) return@LaunchedEffect
        val count = transcript.rows.size
        if (count > 0) listState.animateScrollToItem(count)
    }

    Column(Modifier.fillMaxSize()) {
        Box(Modifier.weight(1f)) {
            if (transcript.rows.isEmpty()) {
                // Whatever this pane is waiting on goes above the empty state,
                // not inside the transcript. On iOS the banner lived in the
                // scroll view, which only exists once there are rows — so a
                // session that never loaded showed "Say something to begin"
                // with the reason it was empty hidden behind the very condition
                // that made it empty.
                AgentEmpty(phase)
            } else {
                LazyColumn(
                    state = listState,
                    contentPadding = PaddingValues(12.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    (phase as? AgentPhase.Failing)?.trouble?.let { trouble ->
                        item {
                            // A stale error banner rather than a blanked
                            // screen: a failed poll is not a disconnection, so
                            // the last known transcript stays up while this
                            // device tries again.
                            Text(
                                trouble.sentence,
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.tertiary,
                            )
                            // Rare by construction, and that is what makes it
                            // bearable at the head of a transcript already
                            // scrolled to its tail: the failure that actually
                            // happens on a phone is a dropped link, which
                            // carries a written sentence and no transcript.
                            trouble.transcript?.let { DetailBox(it) }
                        }
                    }
                    items(transcript.rows.size, key = { transcript.rows[it].id }) { index ->
                        val row = transcript.rows[index]
                        AgentRowView(
                            row = row,
                            isLast = index == transcript.rows.lastIndex,
                            pending = transcript.pendingPermission?.takeIf { pending ->
                                names(pending, row) ||
                                    (row.kind as? com.farcooler.model.TranscriptRow.Kind.Subagent)
                                        ?.block?.children?.any { names(pending, it) } == true
                            },
                            onAnswer = { optionId ->
                                transcript.pendingPermission?.let { stream.answer(it.id, optionId) }
                            },
                        )
                    }
                    // The turn that is still running, one line ahead of what it
                    // has produced.
                    if (isWorking) {
                        item { WorkingRow() }
                    }
                }
            }
        }

        // Everything that sits over the bottom of the conversation, in one
        // place: the plan and the queue are ATTACHED to the composer rather
        // than scattered around the screen. They are all "what happens next",
        // and the plan — furthest away when it was pinned at the top — is the
        // one the next message is most likely to change.
        Column(
            Modifier.padding(horizontal = 10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (transcript.plan.isNotEmpty()) {
                PlanPanel(transcript.plan)
            }
            for (queued in transcript.queue) {
                QueuedRow(
                    queued = queued,
                    onEdit = { text -> stream.editQueued(queued.id, text) },
                    onCancel = { stream.cancelQueued(queued.id) },
                    onSteer = { stream.steerQueued(queued.id) },
                )
            }
            transcript.pendingPermission
                ?.takeIf { pending -> transcript.rows.none { attached(pending, it) } }
                ?.let { pending ->
                    ApprovalCard(pending) { optionId -> stream.answer(pending.id, optionId) }
                }

            AgentComposer(
                configOptions = transcript.configOptions,
                availableModes = transcript.availableModes,
                agentMode = transcript.agentMode,
                availableCommands = transcript.availableCommands,
                contextFraction = transcript.contextFraction,
                harness = harness,
                // Null until a session has said which — see [Transcript.backend].
                backend = transcript.backend,
                isWorking = isWorking,
                workspaceId = ref.workspaceId,
                terminalId = ref.terminalId,
                connection = connection,
                onSetConfig = { id, value -> stream.setConfig(id, value) },
                onSetMode = { mode -> stream.setMode(mode) },
                onCancel = { stream.cancel() },
                onSend = { text, images -> stream.send(text, images) },
            )
        }
    }
}

/** Whether any row — top level or inside a block — already shows this request. */
private fun attached(
    pending: com.farcooler.model.PendingPermission,
    row: com.farcooler.model.TranscriptRow,
): Boolean {
    if (names(pending, row)) return true
    val block = (row.kind as? com.farcooler.model.TranscriptRow.Kind.Subagent)?.block
        ?: return false
    return block.children.any { names(pending, it) }
}

/**
 * What an agent pane with nothing in it may claim, as words and one mark.
 *
 * Pure, and separated from the drawing for the reason `a8b13cb` separated the
 * settings copy: **there is no emulator in this program**, so a screen that can
 * only be proved by looking at it cannot be proved at all. Every state below
 * used to be reachable only by owning a runner whose shim was slow or whose
 * link was down — which is most of why they were all drawn the same and nobody
 * could see that they were.
 */
internal data class AgentEmptyState(
    val mark: Mark,
    val title: String,
    val message: String? = null,
    /** What the runner itself said, if it said anything. Never rewritten. */
    val transcript: String? = null,
) {
    enum class Mark {
        /** Still working on it, and no claim either way. */
        SPINNER,

        /** There is no agent here. Quiet, and NEVER red — see [Waited.TOO_LONG]. */
        CHAT,

        /** Still trying, past the point where saying nothing is fair. */
        RETRY,

        /**
         * A failure, and drawn like one.
         *
         * Red rather than amber. A session that would not load is a failure;
         * amber in this app means an agent is waiting on you, and a screen that
         * cannot show you an agent at all is not that.
         */
        ALARM,

        /**
         * No mark and no headline — one quiet sentence, which is all this state
         * has ever wanted to be.
         */
        NONE,
    }
}

/**
 * The four honest states, and the two that are still trying change with how
 * long they have been trying.
 *
 * This screen asked one question — is there a connection error — and had two
 * answers, so it went "Say something to begin." → red failure → transcript, and
 * was wrong at both of the first two. Before the first poll came back it invited
 * a message into a session it knew nothing about, which would not have worked:
 * `AgentSupervisor::send` in `crates/daemon/src/agent_supervisor.rs` looks the
 * terminal up in its writer map and drops the message when no shim is
 * registered. A round trip later it called a shim that was still coming up a
 * failure, and kept calling it one for as long as the shim took.
 *
 * [AgentPhase.Live] with no rows is the one state the invitation was ever true
 * for.
 *
 * The words are `40a6cd1`'s, deliberately: the fact being reported is the same
 * fact on both phones, and this repo has spent several commits this week undoing
 * two apps disagreeing about one. What is NOT shared is the promise neither of
 * them makes — no state here tells anybody to send something to start an agent,
 * because that is the advice that does not work. What starts one is the pane
 * going into agent mode.
 */
internal fun agentEmptyState(phase: AgentPhase): AgentEmptyState = when (phase) {
    // One round trip, usually. Says nothing about whether a session exists,
    // because nothing knows yet.
    is AgentPhase.Opening ->
        AgentEmptyState(AgentEmptyState.Mark.SPINNER, "Loading this session…")

    is AgentPhase.Starting ->
        if (phase.waited == Waited.A_MOMENT) {
            // The Mac's words, not a second set: `AgentComposer.swift` draws
            // "Starting the agent…" for a chat with no rows and no config
            // options, which is this exact fact.
            AgentEmptyState(AgentEmptyState.Mark.SPINNER, "Starting the agent…")
        } else {
            // Still true, still not a failure, and no longer spinning — a
            // spinner that never ends is its own bug.
            AgentEmptyState(
                AgentEmptyState.Mark.CHAT,
                "No agent on this pane yet",
                "This pane hasn’t started one. The conversation appears here as soon as it does.",
            )
        }

    is AgentPhase.Live ->
        AgentEmptyState(AgentEmptyState.Mark.NONE, "Say something to begin.")

    is AgentPhase.Failing -> when (phase.waited) {
        // Where the alarm finally belongs, with the runner's own output
        // unchanged beneath it. Those words are the whole diagnosis of a runner
        // nobody can reach, so they stay — under a sentence rather than
        // standing in for one, which is what [DetailBox] is for.
        Waited.TOO_LONG -> AgentEmptyState(
            AgentEmptyState.Mark.ALARM,
            "Could not load this session",
            phase.trouble.sentence,
            phase.trouble.transcript,
        )

        Waited.A_WHILE -> AgentEmptyState(
            AgentEmptyState.Mark.RETRY, "Still trying", phase.trouble.sentence)

        // A poll that did not come back is not news yet — there is another one
        // 700 ms behind it. The sentence goes under the spinner so the screen is
        // not silent about it either.
        Waited.A_MOMENT -> AgentEmptyState(
            AgentEmptyState.Mark.SPINNER, "Loading this session…", phase.trouble.sentence)
    }
}

/**
 * One full-screen state, composed the way this app composes them.
 *
 * The proportions are [TerminalPane]'s `Status`, which is where every
 * full-screen state in this app has been settled: a mark, 12, a `titleMedium`
 * headline, 6, a `bodyMedium` sentence, and the host's own words in a box
 * below. Not literally shared with it, because that one paints itself onto the
 * terminal's own black and sets its text in white — a chat pane sits on the
 * theme's surface, and the copy would have been the only part worth reusing.
 */
@Composable
private fun AgentEmpty(phase: AgentPhase) {
    val state = agentEmptyState(phase)
    Column(
        Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        when (state.mark) {
            AgentEmptyState.Mark.NONE -> Unit

            AgentEmptyState.Mark.SPINNER -> {
                CircularProgressIndicator(Modifier.size(28.dp), strokeWidth = 2.dp)
                Spacer(Modifier.height(12.dp))
            }

            else -> {
                Icon(
                    when (state.mark) {
                        AgentEmptyState.Mark.CHAT -> Icons.Outlined.ChatBubbleOutline
                        AgentEmptyState.Mark.RETRY -> Icons.Outlined.Refresh
                        else -> Icons.Outlined.Warning
                    },
                    // The headline says it. A mark that repeats its own
                    // headline into a screen reader is one more thing to listen
                    // through before reaching the sentence that matters.
                    contentDescription = null,
                    tint = if (state.mark == AgentEmptyState.Mark.ALARM) {
                        MaterialTheme.colorScheme.error
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                    modifier = Modifier.size(28.dp),
                )
                Spacer(Modifier.height(12.dp))
            }
        }

        if (state.mark == AgentEmptyState.Mark.NONE) {
            // The invitation is not a status. It is one quiet line, the way it
            // has always been drawn here and on iOS, and giving it a headline's
            // weight would make an empty conversation look like a problem.
            Text(
                state.title,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        } else {
            Text(
                state.title,
                style = MaterialTheme.typography.titleMedium,
                textAlign = TextAlign.Center,
            )
        }

        state.message?.let { sentence ->
            Spacer(Modifier.height(6.dp))
            Text(
                sentence,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
        }

        state.transcript?.let { words ->
            Spacer(Modifier.height(10.dp))
            DetailBox(words)
        }
    }
}

/** A turn in progress, said where the work is appearing. */
@Composable
private fun WorkingRow() {
    var dots by remember { mutableStateOf(1) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(400)
            dots = dots % 3 + 1
        }
    }
    Text(
        "Working" + ".".repeat(dots),
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/** A message written but not yet sent. */
@Composable
private fun QueuedRow(
    queued: QueuedPrompt,
    onEdit: (String) -> Unit,
    onCancel: () -> Unit,
    onSteer: () -> Unit,
) {
    var editing by remember { mutableStateOf(false) }
    var draft by remember(queued.id) { mutableStateOf(queued.text) }

    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(14.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(horizontal = 12.dp, vertical = 8.dp),
    ) {
        if (editing) {
            BasicTextField(
                value = draft,
                onValueChange = { draft = it },
                textStyle = LocalTextStyle.current.copy(
                    color = MaterialTheme.colorScheme.onSurface,
                ),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                modifier = Modifier.fillMaxWidth(),
            )
        } else if (queued.text.isEmpty() && queued.imageCount > 0) {
            // An image with no words is still a message. Without this the
            // bubble was empty and read as a dropped attachment.
            Text(
                if (queued.imageCount == 1) "1 image" else "${queued.imageCount} images",
                style = MaterialTheme.typography.bodyMedium,
            )
        } else {
            Text(queued.text, style = MaterialTheme.typography.bodyMedium)
        }

        Row(
            Modifier.padding(top = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                "Queued",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            // The queue's whole point is that a message you can still see and
            // still edit beats one already gone — so waiting is the default.
            // But a message written mid-turn is very often a correction, and a
            // correction is worth nothing once the wrong thing has been done.
            Action("Send now", onSteer)
            Action(if (editing) "Save" else "Edit") {
                if (editing) {
                    editing = false
                    val trimmed = draft.trim()
                    if (trimmed.isNotEmpty() && trimmed != queued.text) onEdit(trimmed)
                } else {
                    draft = queued.text
                    editing = true
                }
            }
            Action("Remove", onCancel)
        }
    }
}

@Composable
private fun Action(label: String, onClick: () -> Unit) {
    Text(
        label,
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.clickable(onClick = onClick),
    )
}

/**
 * The prompt field, plus everything that hangs off it: slash commands, file
 * mentions, image attachments, and every selector the agent advertises.
 *
 * One card, the same rule the Mac's constraint file states: no second header,
 * no footer — whatever this pane needs to say lives here or nowhere.
 */
@Composable
private fun AgentComposer(
    configOptions: List<ConfigOption>,
    availableModes: List<AgentChoice>,
    agentMode: String?,
    availableCommands: List<AgentChoice>,
    contextFraction: Double?,
    harness: String,
    /**
     * Which protocol is carrying this conversation — `acp`, `claude` or
     * `codex` — or null until a session has said which.
     *
     * Straight off [com.farcooler.model.Transcript.backend], which is
     * `SessionStarted.backend` on the wire. Transcribed from the producers
     * rather than assumed: the field is declared in
     * `crates/agent-core/src/event.rs` as a plain `String` with
     * `#[serde(default = "acp_backend")]` and NO `skip_serializing_if`, so it
     * is always present going out; the three writers each pass
     * `BackendKind::as_str()`, which is exactly `"acp"`, `"claude"` or
     * `"codex"`. It reaches a phone verbatim — `ffi.rs` copies `payload_json`
     * into the `payloadJson` string this app decodes and touches nothing
     * inside it. No new field.
     */
    backend: String?,
    isWorking: Boolean,
    workspaceId: String,
    /** This pane, so it can pick up anything another pane has left for it. */
    terminalId: String,
    connection: Connection,
    onSetConfig: (String, String) -> Unit,
    onSetMode: (String) -> Unit,
    onCancel: () -> Unit,
    onSend: (String, List<AgentStream.Attachment>) -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    // Saveable, because a half-typed message is exactly what
    // `docs/jobs-to-be-done.md` F4 says the phone must not lose: a process
    // killed in a pocket between sets should not cost somebody the sentence
    // they were writing. `TextFieldValue.Saver` carries the selection with the
    // text, so the caret comes back where it was too.
    //
    // Keyed to nothing, and that is now correct rather than a drift.
    //
    // `c37f487` recorded what this said then: one screen was re-pointed at
    // every pane in turn, so one draft was shared across every terminal in the
    // fleet, and a message half typed to one agent appeared in the composer of
    // the next one you opened. The note said it was waiting on the
    // pane-lifetime work, and this is it — the composable itself is per pane
    // now, so "keyed to nothing" means keyed to this pane.
    //
    // [WorkspaceScreen] wraps each pane in a `SaveableStateHolder` bucketed by
    // `Pane.id`, which does two things no key here could. It makes the
    // separation certain rather than a consequence of how `rememberSaveable`
    // derives a key from the composition's shape; and it KEEPS what is written
    // here when the pane is unmounted by the mount limit, so a draft survives
    // its own tab being evicted — and survives the process being killed, which
    // is what `docs/jobs-to-be-done.md` F4 actually asks for.
    var field by rememberSaveable(stateSaver = TextFieldValue.Saver) {
        mutableStateOf(TextFieldValue(""))
    }
    // Notes the review pane has put here for this agent.
    //
    // APPENDED, never assigned over. This field is where a half-typed message
    // lives, and that message is the one thing on this pane a person wrote —
    // dropping a batch of review notes on top of it would be the app throwing
    // away a sentence to make room for another one. The same rule
    // `ComposerHandoff.offer` follows when two batches arrive before either is
    // taken, and for the same reason.
    //
    // Taken once and cleared inside the handoff, so a recomposition cannot
    // append it twice. The caret goes to the end, which is where somebody who
    // wants to add a line to what just arrived would put it — and it is also why
    // the arriving text goes at the END rather than the start: the notes are the
    // thing about to be sent, and the draft above them is context.
    val waiting by connection.composerHandoff.waiting.collectAsStateWithLifecycle()
    LaunchedEffect(waiting[terminalId]) {
        val text = connection.composerHandoff.take(terminalId) ?: return@LaunchedEffect
        val joined = if (field.text.isEmpty()) text else field.text.trimEnd() + "\n\n" + text
        field = TextFieldValue(joined, TextRange(joined.length))
    }

    var mentionResults by remember { mutableStateOf<List<String>>(emptyList()) }
    var attachments by remember { mutableStateOf<List<AgentStream.Attachment>>(emptyList()) }
    var attachmentError by remember { mutableStateOf<String?>(null) }
    var overflowOpen by remember { mutableStateOf(false) }

    val token = remember(field) { activeToken(field.text, field.selection.start) }

    val picker = rememberLauncherForActivityResult(
        ActivityResultContracts.PickVisualMedia()
    ) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        // Loudly, not silently. A photo that cannot be read looks exactly like
        // a picker that did nothing.
        val bytes = runCatching {
            context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
        }.getOrNull()
        if (bytes == null || bytes.isEmpty()) {
            attachmentError = "That image could not be read."
            return@rememberLauncherForActivityResult
        }
        // PNG only when it really is one — a picker hands back HEIC as often as
        // anything else, and telling the agent the wrong type fails at the far
        // end.
        val mime =
            if (bytes.size > 4 && bytes[0] == 0x89.toByte() && bytes[1] == 'P'.code.toByte()) {
                "image/png"
            } else {
                context.contentResolver.getType(uri) ?: "image/jpeg"
            }
        attachments = attachments + AgentStream.Attachment(mime, bytes)
        attachmentError = null
    }

    // Debounced rather than fired on every keystroke: each search is an SSH
    // round trip, and one whose result arrives after the next keystroke already
    // superseded it is nothing but cost.
    LaunchedEffect(token) {
        val mention = token as? ComposerToken.Mention
        if (mention == null) {
            mentionResults = emptyList()
            return@LaunchedEffect
        }
        delay(200)
        val data = runCatching {
            connection.core.call(
                "worktree.file_search",
                Connection.args(
                    "workspace" to workspaceId,
                    "query" to mention.prefix,
                    "limit" to 20,
                ),
            )
        }.getOrNull() ?: return@LaunchedEffect
        mentionResults = data["paths"]?.jsonArray
            ?.mapNotNull { it.jsonPrimitive.contentOrNull } ?: emptyList()
    }

    fun apply(range: IntRange, replacement: String) {
        val before = field.text.substring(0, range.first)
        val after = field.text.substring(minOf(range.last + 1, field.text.length))
        val text = before + replacement + after
        field = TextFieldValue(text, TextRange(before.length + replacement.length))
        mentionResults = emptyList()
    }

    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(24.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        // The suggestions, above what they complete.
        when (val current = token) {
            is ComposerToken.Slash -> {
                val matches = availableCommands.filter {
                    current.prefix.isEmpty() ||
                        it.name.lowercase().startsWith(current.prefix.lowercase())
                }
                if (matches.isNotEmpty()) {
                    SuggestionList(matches) { apply(current.range, "/$it ") }
                }
            }

            is ComposerToken.Mention -> {
                if (mentionResults.isNotEmpty()) {
                    SuggestionList(mentionResults.map { AgentChoice(it, it) }) {
                        apply(current.range, "@$it ")
                    }
                }
            }

            ComposerToken.None -> Unit
        }

        if (attachments.isNotEmpty()) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                attachments.forEachIndexed { index, attachment ->
                    Row(
                        Modifier
                            .clip(RoundedCornerShape(8.dp))
                            .background(MaterialTheme.colorScheme.surfaceContainerHighest)
                            .padding(horizontal = 8.dp, vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text("Image ${index + 1}", style = MaterialTheme.typography.labelSmall)
                        Spacer(Modifier.width(4.dp))
                        Icon(
                            Icons.Filled.Close,
                            contentDescription = "Remove",
                            modifier = Modifier
                                .size(14.dp)
                                .clickable {
                                    attachments = attachments.filterIndexed { i, _ -> i != index }
                                },
                        )
                    }
                }
            }
        }

        attachmentError?.let {
            Text(
                it,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.tertiary,
                modifier = Modifier.clickable { attachmentError = null },
            )
        }

        // How full the context window is, when the agent has said. The Mac
        // shows this and neither phone app ever did — which meant the one
        // number that tells you a long session is about to start forgetting was
        // visible on a laptop and nowhere else.
        contextFraction?.let { fraction ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                LinearProgressIndicator(
                    progress = { fraction.toFloat() },
                    modifier = Modifier.weight(1f).height(3.dp),
                )
                Spacer(Modifier.width(8.dp))
                Text(
                    "${(fraction * 100).toInt()}% context",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        // Settings on top, the message and its send button beneath. The other
        // way round leaves the field stranded at the top of the card with a
        // band of dead space under it, and puts the send button at the end of a
        // row of small secondary controls rather than beside the thing it
        // sends.
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(
                onClick = {
                    picker.launch(
                        PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)
                    )
                },
                modifier = Modifier.size(32.dp),
            ) {
                Icon(
                    Icons.Filled.AddPhotoAlternate,
                    contentDescription = "Attach an image",
                    modifier = Modifier.size(18.dp),
                )
            }

            // Mode, model and effort get a permanent place: what the agent is
            // allowed to do without asking, what it costs, and how hard it
            // tries. All three get reached for mid-session. Everything else
            // folds into the menu beside them.
            val inline = INLINE_IDS.mapNotNull { id -> configOptions.firstOrNull { it.id == id } }
            val folded = configOptions.filterNot { it.id in INLINE_IDS }

            Row(
                Modifier.weight(1f),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                for (option in inline) {
                    SelectorChip(option, onSetConfig)
                }
            }

            if (configOptions.isNotEmpty() || availableModes.size > 1) {
                Box {
                    IconButton(
                        onClick = { overflowOpen = true },
                        modifier = Modifier.size(32.dp),
                    ) {
                        Icon(
                            Icons.Filled.MoreHoriz,
                            contentDescription = "More settings",
                            modifier = Modifier.size(18.dp),
                        )
                    }
                    DropdownMenu(overflowOpen, onDismissRequest = { overflowOpen = false }) {
                        val shown = folded.ifEmpty { configOptions }
                        for (option in shown) {
                            // "Model · Sonnet" — the question and its answer, so
                            // a menu of five reads as five settings rather than
                            // five words.
                            val current =
                                option.options.firstOrNull { it.id == option.currentValue }?.name
                            Text(
                                if (current != null) "${option.name} · $current" else option.name,
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                            )
                            for (choice in option.options) {
                                DropdownMenuItem(
                                    text = { Text(choice.name) },
                                    trailingIcon = {
                                        if (choice.id == option.currentValue) {
                                            Icon(
                                                Icons.Filled.Check,
                                                contentDescription = "Selected",
                                                modifier = Modifier.size(16.dp),
                                            )
                                        }
                                    },
                                    onClick = {
                                        overflowOpen = false
                                        onSetConfig(option.id, choice.id)
                                    },
                                )
                            }
                            HorizontalDivider()
                        }
                        // An older daemon that only reports modes still gets a
                        // picker.
                        if (configOptions.isEmpty()) {
                            for (mode in availableModes) {
                                DropdownMenuItem(
                                    text = { Text(mode.name) },
                                    onClick = {
                                        overflowOpen = false
                                        onSetMode(mode.id)
                                    },
                                )
                            }
                        }
                    }
                }
            }

            AdapterBadge(backend)
        }

        Row(verticalAlignment = Alignment.Bottom) {
            Box(Modifier.weight(1f)) {
                if (field.text.isEmpty()) {
                    Text(
                        "Message $harness",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                BasicTextField(
                    value = field,
                    onValueChange = { field = it },
                    textStyle = MaterialTheme.typography.bodyMedium.copy(
                        color = MaterialTheme.colorScheme.onSurface,
                    ),
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                    modifier = Modifier.fillMaxWidth().heightIn(max = 140.dp),
                )
            }
            Spacer(Modifier.width(8.dp))

            // Stop, while a turn is running. The Mac has had this since the
            // native agent view landed and neither phone app wired it up, so an
            // agent that had gone off in the wrong direction could only be
            // stopped by switching the pane back to a terminal and pressing
            // Ctrl-C — the one thing a chat surface is supposed to make
            // unnecessary.
            if (isWorking) {
                IconButton(onClick = onCancel) {
                    Icon(Icons.Filled.Stop, contentDescription = "Stop this turn")
                }
            }

            IconButton(
                onClick = {
                    val message = field.text.trim()
                    if (message.isEmpty() && attachments.isEmpty()) return@IconButton
                    onSend(message, attachments)
                    field = TextFieldValue("")
                    attachments = emptyList()
                    mentionResults = emptyList()
                },
                enabled = field.text.isNotBlank() || attachments.isNotEmpty(),
            ) {
                Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Send")
            }
        }
    }
}

/**
 * The Mac's rule, in the Mac's words: anything that is not `acp` is a native
 * backend.
 *
 * "ACP" stays capitalized — it is an acronym, Agent Client Protocol, and
 * lowercasing it makes a proper noun look like a status word. `cb13d31`'s
 * sentence case is about sentences and labels, not about spelling a name wrong.
 *
 * Null in, null out: a pane nobody has heard from names no protocol.
 */
internal fun adapterBadgeLabel(backend: String?): String? = when {
    backend.isNullOrEmpty() -> null
    backend != "acp" -> "Native"
    else -> "ACP"
}

/** What a screen reader is told, since two words on their own explain nothing. */
internal fun adapterBadgeDescription(backend: String?): String? = when (adapterBadgeLabel(backend)) {
    "Native" -> "Native: driven through $backend’s own protocol, with no adapter"
    "ACP" -> "ACP: driven through an Agent Client Protocol adapter"
    else -> null
}

/**
 * Which protocol is carrying this chat, at the far end of the row that says what
 * the next message costs.
 *
 * HERE because this surface has no header to put it in. The Mac draws it in the
 * pane's title bar beside the pane's name (`TileView.headerContent`) and a
 * phone's agent pane deliberately has none — "one card, no second header, no
 * footer" is the rule this composer already states, and it is why the mode and
 * the attachments live in here at all. So the nearest true equivalent of
 * "beside the name" is the composer: the placeholder directly below reads
 * "Message Claude", and the badge above it finishes that sentence with which
 * protocol Claude is on. It is also the one piece of chrome on screen for the
 * whole life of this surface, including while the transcript is still empty —
 * which is exactly when somebody asking "why is this behaving oddly" is looking.
 *
 * PAST THE SELECTORS AND WITHOUT A CAPSULE, because it is not a control. The
 * chips beside it are: [SelectorChip] wears a capsule and opens a menu, and a
 * capsule here would promise a menu that does not exist. The selector row takes
 * `weight(1f)`, so this needs no spacer of its own — the chips give back
 * whatever they do not use and the badge lands against the trailing edge.
 *
 * The COLOR is a hierarchy step rather than the Mac's accent, and that is a
 * platform decision rather than drift. `TileView` sets Native in
 * `Color.accentColor` over a wash of it; this pane sits on a theme-chosen
 * ground, and `ChangesScreen`'s rule — no accent text anywhere, because how
 * well eleven points of blue reads depends on which theme is in force — is the
 * finding `353cd80` landed. Every other thing in this row is already a
 * secondary label besides, so accent here would read as the one link among
 * them. Native takes the surface's own foreground and ACP the muted one, which
 * is the same emphasis said in a way this theme can say it.
 */
@Composable
private fun AdapterBadge(backend: String?) {
    val label = adapterBadgeLabel(backend) ?: return
    val description = adapterBadgeDescription(backend)
    Text(
        label,
        style = MaterialTheme.typography.labelSmall,
        color = if (label == "Native") MaterialTheme.colorScheme.onSurface
        else MaterialTheme.colorScheme.onSurfaceVariant,
        // Never the thing that gets truncated: the chips beside it carry
        // agent-chosen names of any length and already ellipsize, and this is
        // three to six characters.
        maxLines = 1,
        modifier = Modifier
            .padding(start = 8.dp)
            .semantics { if (description != null) contentDescription = description },
    )
}

private val INLINE_IDS = listOf("mode", "model", "effort")

@Composable
private fun SelectorChip(option: ConfigOption, onSetConfig: (String, String) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val current = option.options.firstOrNull { it.id == option.currentValue }?.name ?: option.name
    Box {
        Text(
            current,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier
                .clip(RoundedCornerShape(50))
                .background(MaterialTheme.colorScheme.surfaceContainerHighest)
                .clickable { open = true }
                .padding(horizontal = 9.dp, vertical = 3.dp),
        )
        DropdownMenu(open, onDismissRequest = { open = false }) {
            for (choice in option.options) {
                DropdownMenuItem(
                    text = { Text(choice.name) },
                    onClick = {
                        open = false
                        onSetConfig(option.id, choice.id)
                    },
                )
            }
        }
    }
}

/** The list a slash command or an `@` mention pops open, tap to accept. */
@Composable
private fun SuggestionList(items: List<AgentChoice>, onChoose: (String) -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .heightIn(max = 200.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHighest),
    ) {
        LazyColumn {
            val shown = items.take(8)
            items(shown.size) { index ->
                val item = shown[index]
                Column(
                    Modifier
                        .fillMaxWidth()
                        .clickable { onChoose(item.name) }
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                ) {
                    Text(
                        item.name,
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                    )
                    // What it does. The adapter has always sent this and iOS
                    // threw it away, so the list named commands without saying
                    // what any of them were for.
                    if (item.description.isNotEmpty()) {
                        Text(
                            item.description,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                HorizontalDivider()
            }
        }
    }
}
