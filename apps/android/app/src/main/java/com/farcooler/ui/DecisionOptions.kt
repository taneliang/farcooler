package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import com.farcooler.model.NeedsYouAction

/**
 * A decision's options as a radio list, each on its own row with its whole
 * text wrapping, and Answer under it (ov-431). Choosing sends nothing;
 * Answer is off until one is chosen and then sends the chosen option's id,
 * which is what the buttons sent. [sending] is the id on its way, shown as a
 * spinner beside Answer.
 */
@Composable
internal fun DecisionOptions(
    options: List<NeedsYouAction>,
    sending: String?,
    onAnswer: (NeedsYouAction) -> Unit,
    modifier: Modifier = Modifier,
) {
    var picked by rememberSaveable(options.map { it.id }) { mutableStateOf<String?>(null) }
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Column(Modifier.selectableGroup()) {
            options.forEach { option ->
                Row(
                    Modifier
                        .fillMaxWidth()
                        .selectable(
                            selected = picked == option.id,
                            enabled = sending == null,
                            role = Role.RadioButton,
                            onClick = { picked = option.id },
                        )
                        .testTag("decision-option-${option.id}"),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = picked == option.id, onClick = null, enabled = sending == null)
                    Text(
                        option.title.ifBlank { option.id },
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.weight(1f).padding(start = 4.dp, top = 8.dp, bottom = 8.dp),
                    )
                }
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(
                onClick = { options.firstOrNull { it.id == picked }?.let(onAnswer) },
                enabled = picked != null && sending == null,
                modifier = Modifier.testTag("decision-answer"),
            ) { Text("Answer") }
            if (sending != null) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
        }
    }
}
