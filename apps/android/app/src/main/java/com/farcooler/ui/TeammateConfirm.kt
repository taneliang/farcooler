package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.farcooler.ceremony.RunnerRow
import com.farcooler.ceremony.TeammateChoice
import com.farcooler.ceremony.TeammateNames
import com.farcooler.ceremony.ExistingPerson
import com.farcooler.ceremony.TeammateAccessRules
import com.farcooler.model.PersonAccess

/**
 * The confirmation for a teammate's device (ov-854), in the Mac's words: who
 * the directory says they are, the key to compare in person, what they'll be
 * called and may do, and the runners.
 *
 * The email is the directory's, never the code's: the code only named an
 * account, and anyone can show a code naming any account. What stops a
 * stranger is the person adding them checking it's really them, and that the
 * key matches the one under their code.
 */
@Composable
fun TeammateConfirmScreen(
    deviceName: String,
    teammate: TeammateChoice,
    fingerprint: String?,
    publicKey: String,
    rows: List<RunnerRow>,
    /** The people this device could be added to, with the runner each is on. */
    candidates: List<Pair<ExistingPerson, String>> = emptyList(),
    /** What the Access menu offers: Admin only when this phone is the Owner on every runner being added to. */
    choices: List<PersonAccess> = PersonAccess.choices(PersonAccess.ADMIN),
    onToggle: (String) -> Unit,
    onChange: ((TeammateChoice) -> TeammateChoice) -> Unit,
    onConfirm: () -> Unit,
    onCancel: () -> Unit,
) {
    Column(
        Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text(
            "Add your teammate’s device?",
            style = MaterialTheme.typography.titleMedium,
            modifier = Modifier.testTag("teammate-title"),
        )
        Text(
            "“${deviceName.ifBlank { "This device" }}” is signed in as ${teammate.who.email}, who’s on your " +
                "team “${teammate.who.team}”. Add it only if they’re with you and the key below matches " +
                "the one under their code.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.testTag("teammate-subtitle"),
        )

        // Open: comparing it is what the sentence above asks for.
        fingerprint?.let { FingerprintRow(it, publicKey, opened = true) }

        val existing = candidates.firstOrNull { it.first.handle == teammate.person }
        val keeping = existing != null
        if (candidates.isNotEmpty()) {
            val options = candidates.map { it.first.handle to it.first.name } + (null to "New person…")
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().testTag("teammate-person")) {
                options.forEachIndexed { index, (handle, title) ->
                    SegmentedButton(
                        selected = handle == teammate.person,
                        onClick = { onChange { it.copy(person = handle) } },
                        shape = SegmentedButtonDefaults.itemShape(index, options.size),
                        modifier = Modifier.testTag("teammate-person-${handle ?: "new"}"),
                    ) { Text(title, maxLines = 1) }
                }
            }
        }
        if (!keeping) {
            OutlinedTextField(
                value = teammate.name,
                onValueChange = { value -> onChange { it.copy(name = value) } },
                label = { Text("Name") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth().testTag("teammate-name"),
            )
            OutlinedTextField(
                value = teammate.handle,
                onValueChange = { value -> onChange { it.copy(handle = value) } },
                label = { Text("Handle") },
                singleLine = true,
                isError = !TeammateNames.isValidHandle(teammate.handle),
                textStyle = MaterialTheme.typography.bodyLarge.copy(fontFamily = FontFamily.Monospace),
                supportingText = {
                    if (!TeammateNames.isValidHandle(teammate.handle)) {
                        Text(
                            "Use lowercase letters, numbers, and hyphens, starting with a letter.",
                            modifier = Modifier.testTag("teammate-handle-note"),
                        )
                    }
                },
                modifier = Modifier.fillMaxWidth().testTag("teammate-handle"),
            )
        }
        if (existing != null) {
            Text(
                TeammateAccessRules.keeps(existing.first, existing.second),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.testTag("teammate-access-detail"),
            )
        } else {
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().testTag("teammate-access")) {
                choices.forEachIndexed { index, access ->
                    SegmentedButton(
                        selected = access == teammate.access,
                        onClick = { onChange { it.copy(access = access) } },
                        shape = SegmentedButtonDefaults.itemShape(index, choices.size),
                        modifier = Modifier.testTag("teammate-access-${access.wire}"),
                    ) { Text(access.title) }
                }
            }
            Text(
                teammate.access.summary,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.testTag("teammate-access-detail"),
            )
        }

        Column {
            for (row in rows) {
                val enabled = row.blocked == null
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clickable(enabled = enabled) { onToggle(row.runner.id) }
                        .padding(vertical = 6.dp)
                        .testTag("teammate-runner-${row.label}"),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Checkbox(
                        checked = row.picked,
                        onCheckedChange = { onToggle(row.runner.id) },
                        enabled = enabled,
                    )
                    Column(Modifier.weight(1f)) {
                        Text(row.label, style = MaterialTheme.typography.bodyMedium)
                        Text(
                            row.blocked ?: row.detail,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }

        Text(
            "Far Cooler adds this device’s key to ~/.ssh/authorized_keys on each selected runner and " +
                "records it as your teammate’s. Work there runs on the Claude account signed in on " +
                "that runner, so its usage and limits are shared.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Button(
            onClick = onConfirm,
            enabled = rows.any { it.picked } && teammate.isComplete,
            modifier = Modifier.fillMaxWidth().testTag("teammate-add"),
        ) { Text("Add teammate") }
        TextButton(
            onClick = onCancel,
            modifier = Modifier.fillMaxWidth().testTag("teammate-cancel"),
        ) { Text("Cancel") }
    }
}
