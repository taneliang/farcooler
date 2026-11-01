package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.GlancePalette
import com.farcooler.model.PageLinks
import com.farcooler.model.PlanCiRead
import com.farcooler.model.PlanCostWords
import com.farcooler.model.PlanLadder
import com.farcooler.model.PlanSections
import com.farcooler.model.PlanWords
import com.farcooler.model.LANDING_EXPLANATION
import com.farcooler.model.LandingWords
import com.farcooler.model.facts
import com.farcooler.model.unbroken

// A landing in Landing (ov-503, design 2; ov-522; ov-872), drawn for the phone: a card headed "Landing 86" with its title
// under it and what it carries, the ladder compactly ("Checking on CI · step 2 of 3"; a long press explains the landing
// and each step), the CI line, and its tickets. A ticket aboard says nothing; it speaks only once landed or when it waits
// on the owner. Amber only for CI that failed, a landing that stalled, and a ticket waiting on the owner, each with words.
// Every word is `PlanSections` and `LandingWords`'; this lays them out.

/** The landing card: tonal, inset from the list's edges. */
@Composable
fun PlanLandingCard(card: PlanSections.LandingCard, ci: PlanCiRead?, now: Long, onOpenTicket: (String) -> Unit, modifier: Modifier = Modifier) {
    val landing = card.landing
    val amber = glanceColor(GlancePalette.amber)
    val phaseAmber = card.phase.needsAttention || LandingWords.needsAttention(landing, ci)
    val facts = LandingWords.facts(card, ci, now)
    val run = ci?.url?.takeIf { PageLinks.https(it) != null }
    val uri = LocalUriHandler.current
    val step = PlanLadder.step(card.phase)
    val spoken = listOfNotNull(
        landing.label, landing.subtitle, landing.carries,
        card.phase.word + if (step >= PlanLadder.steps.size) "" else ", step ${step + 1} of ${PlanLadder.steps.size}",
    ).joinToString(", ")
    // A long press says what a landing is, then each step of the ladder, the one it stands on in the phase's own words.
    val explanation = listOf(LANDING_EXPLANATION) + PlanLadder.steps.indices.map { index ->
        "${PlanLadder.label(index, step, card.phase)}: ${PlanLadder.explanation(index, step, card.phase)}"
    }
    ExplainedOnLongPress(explanation, modifier.fillMaxWidth()) {
    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp).testTag("plan-landing-${landing.number}"),
    ) {
        Column(Modifier.padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Column(
                Modifier.padding(horizontal = 16.dp).semantics(mergeDescendants = true) { contentDescription = spoken },
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        landing.label, style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold), maxLines = 2,
                        overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-landing-${landing.number}-label"),
                    )
                    landing.subtitle?.let {
                        Text(
                            it, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.testTag("plan-landing-${landing.number}-subtitle"),
                        )
                    }
                    landing.carries?.let {
                        Text(
                            it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-landing-${landing.number}-carries"),
                        )
                    }
                }
                PlanLadderBar(card, phaseAmber)
                if (facts.isNotEmpty()) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = (if (run != null) Modifier.clickable(role = Role.Button) { uri.openUri(run) } else Modifier)
                            .heightIn(min = if (run != null) 44.dp else 0.dp),
                    ) {
                        Text(
                            LandingWords.unbroken(facts),
                            style = MaterialTheme.typography.bodySmall.copy(fontWeight = if (phaseAmber) FontWeight.Medium else FontWeight.Normal),
                            color = if (phaseAmber) amber else MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 2, overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f, fill = false).testTag("plan-landing-${landing.number}-words"),
                        )
                        if (run != null) {
                            Spacer(Modifier.width(6.dp))
                            Icon(Icons.AutoMirrored.Outlined.OpenInNew, contentDescription = null, modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.outline)
                        }
                    }
                }
            }
            card.tickets.forEach { PlanLandingTicketRow(it, now, onClick = { onOpenTicket(it.id) }) }
        }
    }
    }
}

/** Three segments, the steps behind filled, the one it's on in ink (amber when it needs you), and the phase in words under them. */
@Composable
private fun PlanLadderBar(card: PlanSections.LandingCard, attention: Boolean) {
    val current = PlanLadder.step(card.phase)
    val scheme = MaterialTheme.colorScheme
    val amber = glanceColor(GlancePalette.amber)
    Column(verticalArrangement = Arrangement.spacedBy(4.dp), modifier = Modifier.testTag("plan-ladder")) {
        Row(horizontalArrangement = Arrangement.spacedBy(3.dp), modifier = Modifier.fillMaxWidth()) {
            for (index in PlanLadder.steps.indices) {
                Box(
                    Modifier.weight(1f).height(4.dp).clip(CircleShape).background(
                        when {
                            index < current -> scheme.onSurfaceVariant
                            index == current -> if (attention) amber else scheme.onSurface
                            else -> scheme.outlineVariant
                        },
                    ),
                )
            }
        }
        Text(
            PlanLadder.compact(card.phase),
            style = MaterialTheme.typography.labelLarge,
            color = if (attention) amber else scheme.onSurface,
            maxLines = 1, overflow = TextOverflow.Ellipsis,
            modifier = Modifier.testTag("plan-landing-${card.landing.number}-phase"),
        )
    }
}

/** One ticket inside a landing: its title, how it stands, and what it says in amber. */
@Composable
private fun PlanLandingTicketRow(row: PlanSections.LandingTicket, now: Long, onClick: () -> Unit) {
    val amber = glanceColor(GlancePalette.amber)
    // "Needs you" is the ticket's note, said once, in amber; a ticket simply aboard says nothing (ov-522).
    val warning = if (row.waitsOnOwner) null else PlanWords.stale(row.ticket, now)
    val overBudget = PlanCostWords.overBudget(row.ticket)
    val standing = row.note.orEmpty()
    val attention = row.noteNeedsAttention || warning != null || overBudget != null
    val spoken = listOfNotNull(row.ticket.heading, standing.ifEmpty { null }, warning, PlanCostWords.overBudgetSpoken(row.ticket)).joinToString(", ")
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 4.dp).testTag("plan-landing-ticket-${row.key}")
            .semantics(mergeDescendants = true) { contentDescription = spoken },
    ) {
        Column(Modifier.weight(1f)) {
            Text(row.ticket.heading, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (standing.isNotEmpty()) {
                Text(
                    standing, style = MaterialTheme.typography.bodySmall,
                    color = if (row.noteNeedsAttention) amber else MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-landing-ticket-${row.key}-note"),
                )
            }
            if (warning != null) Text(warning, style = MaterialTheme.typography.labelMedium, color = amber)
            if (overBudget != null) Text(overBudget, style = MaterialTheme.typography.labelMedium, color = amber)
        }
        if (attention) {
            Spacer(Modifier.width(8.dp))
            Box(Modifier.size(6.dp).clip(CircleShape).background(amber))
        }
    }
}
