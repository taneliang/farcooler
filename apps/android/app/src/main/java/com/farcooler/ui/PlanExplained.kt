package com.farcooler.ui

import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup

// A long press explains (ov-522): what a train is, or what each step of the ladder means, in lines a person can read
// in a breath. The Mac says the same words as a tooltip; a phone has no hover, so the press shows them in a popup, and
// TalkBack offers "Explain".

/** [content], which a long press explains with [lines]: the first is the headline, the rest follow in a quieter voice. */
@Composable
fun ExplainedOnLongPress(lines: List<String>, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    var shown by remember { mutableStateOf(false) }
    Box(
        modifier
            .pointerInput(Unit) { detectTapGestures(onLongPress = { shown = true }) }
            .semantics { customActions = listOf(CustomAccessibilityAction("Explain") { shown = true; true }) },
    ) {
        content()
        if (shown) {
            Popup(onDismissRequest = { shown = false }) {
                Surface(
                    shape = MaterialTheme.shapes.medium,
                    tonalElevation = 6.dp,
                    shadowElevation = 6.dp,
                    modifier = Modifier.widthIn(max = 320.dp).testTag("plan-explanation"),
                ) {
                    Column(Modifier.padding(16.dp), verticalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp)) {
                        lines.forEachIndexed { index, line ->
                            Text(
                                line,
                                style = if (index == 0) MaterialTheme.typography.bodyMedium else MaterialTheme.typography.bodySmall,
                                color = if (index == 0) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        }
    }
}
