package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.farcooler.model.WorkflowChangeReview

/**
 * A workflow change, shown whole, with Approve at the end (ov-978).
 *
 * Every command, environment variable, argument and remote the change names is a row of its own,
 * wrapped and never cut off, so a person weighs all of it before the one button that publishes it.
 * Approve is the last thing on the screen, below "In full, exactly as it runs", and sends the words
 * the daemon asked for. A row or a notification can't: they never show these parts.
 */
@Composable
internal fun WorkflowChangeReviewDialog(
    review: WorkflowChangeReview,
    sending: Boolean,
    refusal: String?,
    onApprove: (String) -> Unit,
    onDecline: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            WorkflowChangeReviewBody(review, sending, refusal, onApprove, onDecline, onDismiss)
        }
    }
}

/** The review itself, apart from the dialog that holds it, so a capture can draw it. */
@Composable
internal fun WorkflowChangeReviewBody(
    review: WorkflowChangeReview,
    sending: Boolean,
    refusal: String?,
    onApprove: (String) -> Unit,
    onDecline: (String) -> Unit,
    onDismiss: () -> Unit,
) {
            Column(
                Modifier.verticalScroll(rememberScrollState()).padding(16.dp).testTag("workflow-change-review"),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text("Review change", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                    TextButton(onClick = onDismiss) { Text("Close") }
                }
                SelectionContainer {
                    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                        Text(review.headline, style = MaterialTheme.typography.titleMedium)
                        review.intro.forEach { Text(spans(it), style = MaterialTheme.typography.bodyMedium) }
                        review.parts.forEach { part ->
                            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                Text(part.title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                                part.items.forEach { item ->
                                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                        Text("•", color = MaterialTheme.colorScheme.onSurfaceVariant)
                                        Text(spans(item), style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                                    }
                                }
                            }
                        }
                    }
                }
                Text(
                    "Approving publishes exactly what is listed above. Nothing changes until you do.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Button(onClick = { onApprove(review.approval) }, enabled = !sending, modifier = Modifier.testTag("workflow-change-approve")) { Text("Approve") }
                    OutlinedButton(onClick = { onDecline(WorkflowChangeReview.DECLINE) }, enabled = !sending, modifier = Modifier.testTag("workflow-change-decline")) { Text("Decline") }
                    if (sending) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                }
                refusal?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error) }
            }
}

/** A line with its `code` spans in monospace, the backticks dropped. */
internal fun spans(text: String): AnnotatedString = buildAnnotatedString {
    text.split("`").forEachIndexed { i, piece ->
        if (i % 2 == 1) withStyle(SpanStyle(fontFamily = FontFamily.Monospace)) { append(piece) } else append(piece)
    }
}
