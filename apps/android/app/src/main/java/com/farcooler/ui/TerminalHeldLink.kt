package com.farcooler.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalUriHandler
import com.farcooler.model.LinkOpen
import com.farcooler.model.TaskKeyLinks

/**
 * The link a long press on terminal output landed on. A task key's link
 * (ov-215) is titled with the key and opens the task in the app, through
 * [TaskKeyLinker.follow], and never through the system: it would hand
 * `farcooler://` to whichever channel's app claimed it. Any other link is
 * titled with the URL itself, because "Open" without saying which link asks
 * you to trust output an agent produced without showing you what you trust.
 *
 * [onOpen] gets the reason an outside link didn't open, or null.
 */
@Composable
fun HeldLinkDialog(
    link: String,
    linker: TaskKeyLinker,
    failure: String?,
    onOpen: (String?) -> Unit,
    onCopy: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val uri = LocalUriHandler.current
    val key = TaskKeyLinks.parse(link)?.second
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(key ?: "Link") },
        text = {
            Column {
                if (key == null) Text(link)
                failure?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            }
        },
        confirmButton = {
            TextButton(onClick = {
                if (key != null) {
                    linker.follow(link)
                    onOpen(null)
                } else {
                    onOpen(LinkOpen.open(link, uri::openUri))
                }
            }) { Text(if (key != null) "Open task" else "Open") }
        },
        dismissButton = { TextButton(onClick = { onCopy(key ?: link) }) { Text("Copy") } },
    )
}
