package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.RunnerTeam

/**
 * A shared runner's people (ov-784), in its settings: each member with their handle and reps, and for the owner a
 * role picker on each, Member or Maintainer, with what a maintainer can do said underneath. The Mac's and the
 * iPhone's People section, in this platform's sentence case. [setRole] null draws no picker; [changing] is the
 * member whose role is on its way to the runner.
 */
@Composable
fun RunnerPeopleSection(
    team: RunnerTeam,
    changing: String?,
    setRole: ((String, RunnerTeam.Role) -> Unit)?,
) {
    SectionTitle(RunnerTeam.SECTION_TITLE)
    for (member in team.members) {
        Row(
            Modifier.fillMaxWidth().testTag("runner-person-${member.handle}"),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(Modifier.weight(1f)) {
                // Two lines: a name cut short is the one thing that says who the row is.
                Text(member.name, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(
                    RunnerTeam.detail(member, showsRole = setRole == null),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            if (member.handle == team.you) {
                Text("You", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (setRole != null) RolePicker(member, enabled = changing == null) { setRole(member.handle, it) }
        }
    }
    if (setRole != null) {
        Text(
            RunnerTeam.ROLES_PURPOSE,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** One member's role, from a menu: the role as the runner has it, and the two to choose from. */
@Composable
private fun RolePicker(member: RunnerTeam.Member, enabled: Boolean, choose: (RunnerTeam.Role) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val now = member.role ?: RunnerTeam.Role.MEMBER
    Box {
        TextButton(
            onClick = { open = true },
            enabled = enabled,
            modifier = Modifier
                .testTag("runner-person-role-${member.handle}")
                .semantics { contentDescription = "${member.name}’s role: ${now.title}" },
        ) {
            Text(now.title)
            Icon(Icons.Filled.ArrowDropDown, contentDescription = null)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            for (role in RunnerTeam.Role.entries) {
                DropdownMenuItem(
                    text = { Text(role.title) },
                    modifier = Modifier.testTag("runner-person-role-${member.handle}-${role.wire}"),
                    onClick = {
                        open = false
                        if (role != now) choose(role)
                    },
                )
            }
        }
    }
}
