package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.model.AgentConversation
import com.farcooler.net.AgentRowStore
import com.farcooler.net.NativePaneModel
import kotlinx.coroutines.launch

/**
 * The switch in a claude pane's bar: the conversation or the terminal under it
 * (ov-374). Drawn only where the conversation is offered. Always a way back to
 * the terminal; never a blank pane.
 */
@Composable
fun NativeSwitchButton(showing: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier) {
    IconButton(onClick = onClick, modifier = modifier.testTag("native-switch")) {
        Icon(
            if (showing) Icons.Outlined.Terminal else Icons.AutoMirrored.Filled.Chat,
            contentDescription = if (showing) "Show terminal" else "Show conversation",
        )
    }
}

/**
 * The conversation view of a terminal-mode claude pane (ov-374): its rows as
 * the runner's projector folds them, newest at the bottom, and a box to type
 * into.
 *
 * A lazy list keyed by row id, so a long session builds only the rows on screen
 * and a row's change redraws that row alone. Older rows are paged in as the top
 * comes into view.
 *
 * It follows like Messages, and the list is laid out from its bottom
 * (`reverseLayout`) to do it: the newest row is item 0, so a list resting on its
 * tail stays on it as a reply streams in or the keyboard takes room, a list the
 * reader scrolled up is never moved, and a session shorter than the screen sits
 * on the composer. Whether it follows is derived from where the list is, so only
 * a finger changes it; a send brings the message into view, and the way back is
 * Jump to latest.
 *
 * [listState] is the pane's, so scrolling survives the switch to the terminal
 * and back.
 */
@Composable
fun NativeAgentView(
    model: NativePaneModel,
    listState: LazyListState,
    showTerminal: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val shown by model.store.shown.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    // On the tail: the newest item's bottom is the list's bottom.
    val pinned by remember(listState) {
        derivedStateOf { listState.firstVisibleItemIndex == 0 && listState.firstVisibleItemScrollOffset == 0 }
    }

    // A message just sent comes into view, wherever the reader was, as in
    // Messages: what they sent is what they're looking for.
    LaunchedEffect(model.sent) {
        if (model.sent > 0) listState.scrollToItem(0)
    }
    LaunchedEffect(shown.rows.size) { model.settleQueued() }

    Column(modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        // Rows held and the runner not answering: said over them, so stale rows
        // never pass for live ones, and the box waits (`canSend`).
        if (shown.rows.isNotEmpty() && shown.isStale) StaleBanner(shown.phase, showTerminal)

        Box(Modifier.weight(1f).fillMaxWidth()) {
            LazyColumn(
                state = listState,
                reverseLayout = true,
                modifier = Modifier.fillMaxSize().testTag("native-transcript"),
                contentPadding = PaddingValues(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.Top),
            ) {
                // Bottom to top: what is below the transcript, then the rows
                // newest first, then the spinner that pages in older ones.
                val queued = model.queued
                items(queued.indices.reversed().toList(), key = { "queued-$it" }) { QueuedLine(queued[it]) }
                if (model.issue == AgentConversation.SendIssue.Handoff) {
                    item(key = "handoff-issue") { HandoffRow(AgentConversation.HANDOFF, showTerminal) }
                }
                items(shown.rows.asReversed(), key = { it.id }) { row -> NativeRowView(row, showTerminal) }
                if (shown.moreBefore) {
                    item(key = "older") {
                        Box(Modifier.fillMaxWidth().padding(8.dp), contentAlignment = Alignment.Center) {
                            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                        }
                        LaunchedEffect(Unit) { model.loadOlder() }
                    }
                }
            }
            if (shown.rows.isEmpty()) EmptyState(shown.phase)
            if (!pinned) {
                SmallFloatingActionButton(
                    onClick = { scope.launch { listState.animateScrollToItem(0) } },
                    modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp).testTag("native-jump-to-latest"),
                ) { Icon(Icons.Filled.KeyboardArrowDown, contentDescription = "Jump to latest") }
            }
        }
        NativeComposer(model, showTerminal)
    }
}

@Composable
private fun StaleBanner(phase: AgentRowStore.Phase, showTerminal: () -> Unit) {
    val shape = RoundedCornerShape(Radius.medium)
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp)
            .background(MaterialTheme.colorScheme.surfaceContainerHigh, shape)
            .border(1.dp, MaterialTheme.colorScheme.tertiary.copy(alpha = 0.55f), shape)
            .padding(start = 12.dp, top = 4.dp, bottom = 4.dp, end = 4.dp)
            .testTag("native-stale"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            if (phase == AgentRowStore.Phase.Unavailable) AgentConversation.STALE_UNAVAILABLE else AgentConversation.STALE_TROUBLE,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.weight(1f),
        )
        TextButton(onClick = showTerminal) { Text("Show terminal") }
    }
}

@Composable
private fun EmptyState(phase: AgentRowStore.Phase) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        when (phase) {
            AgentRowStore.Phase.Loading, AgentRowStore.Phase.Cached -> CircularProgressIndicator(Modifier.size(28.dp), strokeWidth = 2.dp)
            AgentRowStore.Phase.Live -> Quiet("Nothing in this session yet.")
            AgentRowStore.Phase.Unavailable -> Quiet("This pane’s session can’t be shown here. Use the terminal.")
            is AgentRowStore.Phase.Trouble -> Quiet("Can’t reach the runner. Trying again…")
        }
    }
}

@Composable
private fun Quiet(text: String) =
    Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)

/**
 * The conversation's box (ov-374). Send goes through `terminal.compose`: typed
 * into claude's own box and submitted, or taken by claude's queue while it works
 * (R-29). One line, as on the iPhone; attachments and commands are later cards.
 */
@Composable
fun NativeComposer(model: NativePaneModel, showTerminal: () -> Unit) {
    // Collected for the rows going stale or live, which `canSend` reads off the
    // store and Compose can't see on its own: without it Send stayed enabled over
    // rows the runner had stopped answering for.
    val shown by model.store.shown.collectAsStateWithLifecycle()
    val canSend = !shown.isStale && model.canSend
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp).testTag("native-composer-stack"),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        val issue = model.issue
        if (issue != null && issue != AgentConversation.SendIssue.Handoff) IssueLine(issue, showTerminal) { model.issue = null }
        Surface(
            shape = RoundedCornerShape(Radius.large),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Row(Modifier.padding(start = 4.dp, end = 6.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                TextField(
                    value = model.draft,
                    onValueChange = model::onDraft,
                    placeholder = { Text("Message Claude") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences, imeAction = ImeAction.Send),
                    keyboardActions = KeyboardActions(onSend = { model.send() }),
                    colors = TextFieldDefaults.colors(
                        focusedContainerColor = Color.Transparent,
                        unfocusedContainerColor = Color.Transparent,
                        focusedIndicatorColor = Color.Transparent,
                        unfocusedIndicatorColor = Color.Transparent,
                    ),
                    modifier = Modifier.weight(1f).testTag("native-composer"),
                )
                FilledIconButton(
                    onClick = { model.send() },
                    enabled = canSend,
                    // An edge while it can't send, so it still reads on light paper.
                    modifier = Modifier
                        .size(40.dp)
                        .then(if (canSend) Modifier else Modifier.border(1.dp, MaterialTheme.colorScheme.outline, CircleShape))
                        .testTag("native-send"),
                ) { Icon(Icons.Filled.ArrowUpward, contentDescription = "Send") }
            }
        }
    }
}

@Composable
private fun IssueLine(issue: AgentConversation.SendIssue, showTerminal: () -> Unit, dismiss: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceContainerHigh, RoundedCornerShape(Radius.medium))
            .padding(start = 12.dp, top = 4.dp, bottom = 4.dp, end = 4.dp)
            .testTag("native-send-issue"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        when (issue) {
            AgentConversation.SendIssue.DraftInTerminal -> {
                Text(AgentConversation.DRAFT_IN_TERMINAL, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                TextButton(onClick = showTerminal) { Text("Show terminal") }
            }
            is AgentConversation.SendIssue.Said ->
                Text(issue.words, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
            AgentConversation.SendIssue.Handoff -> Unit
        }
        TextButton(onClick = dismiss) { Text("Dismiss") }
    }
}
