package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.farcooler.model.PrActionFailure
import com.farcooler.model.PrActions
import kotlinx.coroutines.launch

/**
 * Replying to a review thread an agent escalated (ov-317): a dialog to write the reply, then a confirmation that names
 * the pull request and the GitHub login it will be posted as, which the runner reads from its own `gh` before anything
 * is sent (`pr.thread_reply` with `send` false). The reply is the person's own text; the runner posts it. The iPhone's
 * `ThreadReplySheet`.
 */
@Composable
fun ThreadReplyDialog(
    question: String,
    preview: suspend () -> Result<PrActions.ReplyPreview>,
    send: suspend (String, PrActions.ReplyPreview) -> String?,
    onDismiss: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var draft by remember { mutableStateOf("") }
    var checking by remember { mutableStateOf(false) }
    var sending by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<String?>(null) }
    var confirming by remember { mutableStateOf<PrActions.ReplyPreview?>(null) }
    val body = PrActions.reply(draft)

    val shown = confirming
    if (shown != null) {
        AlertDialog(
            onDismissRequest = { if (!sending) confirming = null },
            title = { Text(shown.title) },
            text = { Text("${shown.message}\n\n“${PrActions.excerpt(body.orEmpty())}”") },
            confirmButton = {
                TextButton(
                    enabled = body != null && !sending,
                    onClick = {
                        val text = body ?: return@TextButton
                        sending = true
                        scope.launch {
                            val refused = send(text, shown)
                            sending = false
                            confirming = null
                            if (refused != null) failure = refused else onDismiss()
                        }
                    },
                    modifier = Modifier.testTag("pr-reply-send"),
                ) { Text(PrActions.SEND_REPLY) }
            },
            dismissButton = { TextButton(enabled = !sending, onClick = { confirming = null }) { Text("Cancel") } },
            modifier = Modifier.testTag("pr-reply-confirm"),
        )
    }

    AlertDialog(
        onDismissRequest = { if (!sending) onDismiss() },
        title = { Text("Reply") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(question, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("pr-reply-question"))
                OutlinedTextField(
                    value = draft, onValueChange = { draft = it }, minLines = 3,
                    label = { Text("Your reply") },
                    supportingText = {
                        Text(
                            if (PrActions.length(draft.trim()) > PrActions.REPLY_MAX) "That’s too long to post from here. Shorten it, or reply on GitHub."
                            else "The runner posts it with its GitHub login. You’ll see which before it’s sent.",
                        )
                    },
                    isError = PrActions.length(draft.trim()) > PrActions.REPLY_MAX,
                    modifier = Modifier.fillMaxWidth().heightIn(min = 140.dp).testTag("pr-reply-text"),
                )
                failure?.let { Text(it, style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("pr-reply-failure")) }
            }
        },
        confirmButton = {
            if (checking) {
                CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
            } else {
                TextButton(
                    enabled = body != null,
                    onClick = {
                        checking = true
                        failure = null
                        scope.launch {
                            val read = preview()
                            checking = false
                            read.onSuccess { confirming = it }
                            read.onFailure { failure = (it as? PrActionFailure)?.sentence ?: PrActions.refusal(null, null, "reply") }
                        }
                    },
                    modifier = Modifier.testTag("pr-reply-continue"),
                ) { Text("Continue") }
            }
        },
        dismissButton = { TextButton(onClick = onDismiss, modifier = Modifier.testTag("pr-reply-cancel")) { Text("Cancel") } },
        modifier = Modifier.testTag("pr-reply-dialog"),
    )
}
