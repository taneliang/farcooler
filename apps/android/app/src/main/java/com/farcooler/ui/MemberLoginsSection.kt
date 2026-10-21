package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.farcooler.model.MemberOwnLogins

/**
 * Whether members sign in to each agent with their own account on a shared
 * runner (ov-652): one switch per agent, which only the runner's admins can
 * turn, and everyone else reads. The Mac's and the iPhone's section of the
 * same name, in the same words.
 */
@Composable
fun MemberLoginsSection(
    logins: MemberOwnLogins,
    owner: Boolean,
    changing: Boolean,
    set: (MemberOwnLogins.Agent, Boolean) -> Unit,
) {
    SectionTitle(MemberOwnLogins.SECTION_TITLE)
    for (agent in MemberOwnLogins.Agent.entries) {
        val on = logins.requiresOwn(agent)
        Row(
            Modifier.fillMaxWidth().testTag("member-own-login-${agent.wire}"),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(Modifier.weight(1f)) {
                Text(MemberOwnLogins.toggleTitle(agent))
                Text(
                    MemberOwnLogins.stateLine(agent, on),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            // The switch waits while a change is on its way, so two flips
            // can't land out of order.
            Switch(checked = on, enabled = logins.mayChange && !changing, onCheckedChange = { set(agent, it) })
        }
    }
    Text(
        MemberOwnLogins.footer(owner, logins.mayChange),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}
