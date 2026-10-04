package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.BoardReads
import com.farcooler.model.BoardSummary
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskNoteRow
import com.farcooler.net.Connection

// Unread (ov-104, ov-113), on Android: the board's first section, listing what's
// new to this person, in the Mac's words. A line stays until its ticket is
// opened, and opening a ticket on any of this person's devices clears it on all
// of them, when the runner keeps the state. Mark all as read asks first, and
// says that. What goes in it is `BoardSummary`'s and where it sits is
// `BoardList.unreadEntries`; this file draws.

/**
 * The notes Unread lists, by task: the records of the tickets worth reading
 * ([BoardSummary.noteCandidates]), each read once and remembered until its
 * `updatedAt` moves, and read again when what's read changes.
 */
@Composable
fun rememberUnreadNotes(connection: Connection, board: TaskBoard, reads: BoardReads): Map<String, List<TaskNoteRow>> {
    val cache = remember(connection) { mutableMapOf<String, Pair<Long?, List<TaskNoteRow>>>() }
    var notes by remember(connection, board.rows.firstOrNull()?.workspaceId) { mutableStateOf(emptyMap<String, List<TaskNoteRow>>()) }
    val picked = BoardSummary.noteCandidates(board.rows, reads)
    LaunchedEffect(picked.map { it.id to it.updatedAt }, reads) {
        for (row in picked) {
            val cached = cache[row.id]
            if (cached != null && cached.first == row.updatedAt) continue
            val read = connection.taskNotes(row.id) ?: continue
            cache[row.id] = row.updatedAt to read
        }
        notes = picked.mapNotNull { row -> cache[row.id]?.let { row.id to it.second } }.toMap()
    }
    return notes
}

/** "Unread", how many it lists, and Mark all as read while it lists any. */
@Composable
fun UnreadHeader(entry: BoardListEntry.UnreadHeader, onMarkAll: () -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp)
            .padding(start = 16.dp, end = 8.dp, top = 8.dp)
            .testTag("board-unread"),
    ) {
        Text("Unread", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
        if (entry.count > 0) {
            Spacer(Modifier.width(8.dp))
            Text(
                "${entry.count}",
                style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
                color = MaterialTheme.colorScheme.outline,
            )
        }
        Spacer(Modifier.weight(1f))
        if (entry.offersMarkAll) {
            TextButton(onClick = onMarkAll, modifier = Modifier.heightIn(min = 48.dp).testTag("board-mark-all-read")) {
                Text("Mark all as read")
            }
        }
    }
}

@Composable
fun UnreadNothingRow(entry: BoardListEntry.UnreadNothing) {
    Text(
        entry.text,
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp).testTag("board-unread-empty"),
    )
}

@Composable
fun UnreadGroupRow(entry: BoardListEntry.UnreadGroup) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 2.dp)
            .semantics(mergeDescendants = true) { contentDescription = "${entry.title}, ${entry.count}" },
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(entry.title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(
            "${entry.count}",
            style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
            color = MaterialTheme.colorScheme.outline,
        )
    }
}

/** A line: its key, its title, and what's said under them. 48 dp at least, the whole row a target. */
@Composable
private fun UnreadRow(key: String, title: String, tag: String, onOpen: () -> Unit, detail: @Composable () -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onOpen)
            .heightIn(min = 48.dp)
            .padding(horizontal = 16.dp, vertical = 8.dp)
            .testTag(tag)
            .semantics(mergeDescendants = true) {},
    ) {
        Text(key, style = MaterialTheme.typography.labelSmall, fontFamily = FontFamily.Monospace, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(title, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
        detail()
    }
}

@Composable
fun UnreadLineRow(key: String, title: String, tag: String, whenSaid: String, onOpen: () -> Unit) =
    UnreadRow(key, title, tag, onOpen) {
        Text(whenSaid, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }

@Composable
fun UnreadNoteRow(activity: BoardSummary.Activity, tag: String, onOpen: () -> Unit) =
    UnreadRow(activity.key, activity.title, tag, onOpen) {
        Text(
            "${activity.kind.title}  ${activity.text}",
            style = MaterialTheme.typography.bodySmall,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            activity.foot(System.currentTimeMillis()),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.outline,
        )
    }

@Composable
fun UnreadMoreRow(entry: BoardListEntry.UnreadMore) {
    Text(
        entry.text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
    )
}

/** Mark all as read asks first, and says whether it reaches every device. */
@Composable
fun MarkAllReadDialog(message: String, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Mark all as read?") },
        text = { Text(message) },
        confirmButton = { TextButton(onClick = onConfirm, modifier = Modifier.testTag("board-mark-all-read-confirm")) { Text("Mark as read") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
