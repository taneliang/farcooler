package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.TaskKeyCard

/**
 * A task key's card (ov-299), what a long press on a key shows: the title
 * first and largest, then the key, status, theme and lane on one line, then
 * the intent or latest note. Material's dialog, as the phone's equivalent of
 * the Mac's hovercard and the iPhone's preview, with Open task, and Copy
 * where there's a key to copy.
 */
@Composable
fun TaskKeyCardDialog(
    card: TaskKeyCard,
    onOpen: () -> Unit,
    onCopy: (() -> Unit)?,
    onDismiss: () -> Unit,
    failure: String? = null,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = Modifier.testTag("task-key-card-${card.key}"),
        title = { Text(card.title, maxLines = 3, overflow = TextOverflow.Ellipsis) },
        text = { TaskKeyCardBody(card, failure) },
        confirmButton = { TextButton(onClick = onOpen) { Text("Open task") } },
        dismissButton = onCopy?.let { copy -> { TextButton(onClick = copy) { Text("Copy") } } },
    )
}

/** The card's lines under its title. */
@Composable
fun TaskKeyCardBody(card: TaskKeyCard, failure: String? = null) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(
            "${card.key} · ${card.details}",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (card.excerpt.isNotEmpty()) {
            Text(card.excerpt, style = MaterialTheme.typography.bodyMedium, maxLines = 3, overflow = TextOverflow.Ellipsis)
        }
        failure?.let { Text(it, color = MaterialTheme.colorScheme.error) }
    }
}

/** A key's semantics: its title after it, so TalkBack says what "ov-190" is. */
fun Modifier.taskKeyDescription(card: TaskKeyCard?): Modifier =
    if (card == null) this else semantics { contentDescription = card.accessibilityLabel }
