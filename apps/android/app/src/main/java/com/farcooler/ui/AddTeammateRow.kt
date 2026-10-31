package com.farcooler.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag

/**
 * Add Teammate, from People (ov-865): the scan that opens the teammate's confirmation, as on the iPhone. On a runner
 * nobody shares yet it is People's only row, under a line that says so.
 */
@Composable
fun AddTeammateRow(alone: Boolean, onAddTeammate: () -> Unit) {
    Column {
        if (alone) {
            SectionTitle("People")
            Text("It’s just you here so far.", Modifier.testTag("runner-people-alone"))
        }
        OutlinedButton(onClick = onAddTeammate, modifier = Modifier.testTag("people-add-teammate")) { Text("Add teammate") }
        Text(
            "Scan the code on your teammate’s device to give them access to this runner.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
