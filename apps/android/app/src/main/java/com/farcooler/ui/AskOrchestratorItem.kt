package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.QuestionAnswer
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import com.farcooler.model.AskAboutTask

/**
 * Ask the orchestrator on a task's screen (ov-241): the one thing a task offers
 * for changing it, as the Mac has it.
 *
 * Off, with the sentence under it, when the workspace has no orchestrator
 * running; never hidden, so a person looking for it learns why it can't be
 * used. [notice] is what the last ask left behind when the runner couldn't
 * paste and the reference went to the clipboard. The rules are
 * [AskAboutTask]'s.
 */
fun LazyListScope.askOrchestratorItem(
    available: Boolean,
    notice: String?,
    onAsk: () -> Unit,
) {
    item(key = "ask-orchestrator") {
        val why = if (!available) AskAboutTask.UNAVAILABLE else notice
        ListItem(
            headlineContent = { Text(AskAboutTask.TITLE) },
            supportingContent = why?.let { sentence ->
                { Text(sentence, modifier = Modifier.testTag("ask-orchestrator-note")) }
            },
            leadingContent = {
                Icon(
                    Icons.Outlined.QuestionAnswer,
                    contentDescription = null,
                    tint = if (available) MaterialTheme.colorScheme.onSurface
                    else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            },
            modifier = Modifier
                .then(if (available) Modifier.clickable(onClick = onAsk) else Modifier)
                .testTag("ask-orchestrator"),
        )
    }
}
