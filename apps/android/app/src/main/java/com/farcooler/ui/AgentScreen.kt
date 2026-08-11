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
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
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
import com.farcooler.net.AgentStream
import com.farcooler.net.Connection
import com.farcooler.net.TerminalRef
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive

/**
 * One agent session, as a conversation.
 *
 * The surface the terminal screen swaps in when the daemon reports the pane is
 * hosting a chat. Unlike the Mac there is no tmux rectangle to draw into: this
 * pane is always the whole screen, and the tab strip that lets you leave it
 * lives below it exactly the way it lives below a terminal — the terminal
 * screen owns that, not this.
 */
@Composable
fun AgentScreen(model: AppModel, ref: TerminalRef, connection: Connection) {
    val scope = rememberCoroutineScope()
    val stream = remember(ref.terminalId) {
        AgentStream(ref.terminalId, connection.core, scope)
    }
    DisposableEffect(stream) {
        stream.start()
        onDispose { stream.stop() }
    }

    // A class is not a value, so this is what makes the conversation redraw.
    val revision by stream.revision.collectAsStateWithLifecycle()
    val error by stream.connectionError.collectAsStateWithLifecycle()
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
                // The error goes above the empty state, not inside the
                // transcript. On iOS the banner lived in the scroll view, which
                // only exists once there are rows — so a session that never
                // loaded showed "Say something to begin" with the reason it was
                // empty hidden behind the very condition that made it empty.
                Column(
                    Modifier.fillMaxSize().padding(24.dp),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    if (error != null) {
                        Text(
                            "Could not load this session",
                            style = MaterialTheme.typography.titleMedium,
                        )
                        Spacer(Modifier.height(6.dp))
                        Text(
                            error!!,
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center,
                        )
                    } else {
                        Text(
                            "Say something to begin.",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            } else {
                LazyColumn(
                    state = listState,
                    contentPadding = PaddingValues(12.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    if (error != null) {
                        item {
                            // A stale error banner rather than a blanked
                            // screen: a failed poll is not a disconnection, so
                            // the last known transcript stays up while this
                            // device tries again.
                            Text(
                                error!!,
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.tertiary,
                            )
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
                isWorking = isWorking,
                workspaceId = ref.workspaceId,
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
    isWorking: Boolean,
    workspaceId: String,
    connection: Connection,
    onSetConfig: (String, String) -> Unit,
    onSetMode: (String) -> Unit,
    onCancel: () -> Unit,
    onSend: (String, List<AgentStream.Attachment>) -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    var field by remember { mutableStateOf(TextFieldValue("")) }
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
