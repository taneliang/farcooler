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
import com.farcooler.model.PlanViewLanding
import com.farcooler.model.PlanViewTicket
import com.farcooler.model.PlanWords
import com.farcooler.model.span
import com.farcooler.model.LandingWords
import com.farcooler.model.unbroken

// A landing in Landing (ov-503, design 2; ov-522; ov-872), drawn for the phone: a card headed "Landing 86" with its title
// under it and what it carries, the ladder compactly ("Checking on CI · step 2 of 3"; a long press explains the landing
// and each step), the CI line, and its tickets. A ticket aboard says nothing; it speaks only once landed or when it waits
// on the owner. Amber only for CI that failed, a landing that stalled, and a ticket waiting on the owner, each with words.
// Every word is the core's `PlanView` (ov-1016) but the CI line's, which the plan's CI reads supply; this lays them out.

/** The landing card: tonal, inset from the list's edges. [help] is what a landing is, said first on a long press. */
@Composable
fun PlanLandingCard(
    card: PlanViewLanding,
    ci: PlanCiRead?,
    now: Long,
    onOpenTicket: (String) -> Unit,
    modifier: Modifier = Modifier,
    help: String? = null,
) {
    val amber = glanceColor(GlancePalette.amber)
    val phaseAmber = card.attention || ci?.needsAttention == true
    val facts = landingFacts(card, ci, now)
    val run = ci?.url?.takeIf { PageLinks.https(it) != null }
    val uri = LocalUriHandler.current
    val spoken = listOfNotNull(card.label, card.subtitle, card.carries, card.ladder.compact.replace(" · ", ", ")).joinToString(", ")
    // A long press says what a landing is, then each step of the ladder, the one it stands on in the phase's own words.
    val explanation = listOfNotNull(help) + card.ladder.steps.map { "${it.label}: ${it.explanation}" }
    ExplainedOnLongPress(explanation, modifier.fillMaxWidth()) {
    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp).testTag("plan-landing-${card.number}"),
    ) {
        Column(Modifier.padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Column(
                Modifier.padding(horizontal = 16.dp).semantics(mergeDescendants = true) { contentDescription = spoken },
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        card.label, style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold), maxLines = 2,
                        overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-landing-${card.number}-label"),
                    )
                    card.subtitle?.let {
                        Text(
                            it, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.testTag("plan-landing-${card.number}-subtitle"),
                        )
                    }
                    card.carries?.let {
                        Text(
                            it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-landing-${card.number}-carries"),
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
                            modifier = Modifier.weight(1f, fill = false).testTag("plan-landing-${card.number}-words")
                                .semantics { contentDescription = PlanWords.accessibleModel(facts, card.lander?.model.orEmpty()) },
                        )
                        if (run != null) {
                            Spacer(Modifier.width(6.dp))
                            Icon(Icons.AutoMirrored.Outlined.OpenInNew, contentDescription = null, modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.outline)
                        }
                    }
                }
            }
            card.tickets.forEach { PlanLandingTicketRow(it, onClick = { onOpenTicket(it.id) }) }
        }
    }
    }
}

/**
 * "3f78754f · Passed · 2 of 5 jobs done · 12 min · Lander · Haiku · 1.4M": the card's facts line. The pushed commit, how long
 * the landing has been in its phase and the lander are the core's; CI's own line is what the plan's CI read says of it.
 */
private fun landingFacts(card: PlanViewLanding, ci: PlanCiRead?, now: Long): String {
    val parts = mutableListOf<String>()
    card.pushedSha?.takeIf { it.isNotEmpty() }?.let { parts += it.take(8) }
    if (ci != null) {
        parts += LandingWords.ciSummary(ci)
        LandingWords.ciStale(ci, now)?.let { parts += it }
    } else {
        card.ciUnread?.let { parts += it }
    }
    if (card.since > 0) parts += PlanWords.span(now - card.since)
    card.landerWords?.let { parts += it }
    return parts.joinToString(" · ")
}

/** Three segments, the steps behind filled, the one it's on in ink (amber when it needs you), and the phase in words under them. */
@Composable
private fun PlanLadderBar(card: PlanViewLanding, attention: Boolean) {
    val current = card.ladder.at.toInt()
    val scheme = MaterialTheme.colorScheme
    val amber = glanceColor(GlancePalette.amber)
    Column(verticalArrangement = Arrangement.spacedBy(4.dp), modifier = Modifier.testTag("plan-ladder")) {
        Row(horizontalArrangement = Arrangement.spacedBy(3.dp), modifier = Modifier.fillMaxWidth()) {
            for (index in card.ladder.steps.indices) {
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
            card.ladder.compact,
            style = MaterialTheme.typography.labelLarge,
            color = if (attention) amber else scheme.onSurface,
            maxLines = 1, overflow = TextOverflow.Ellipsis,
            modifier = Modifier.testTag("plan-landing-${card.number}-phase"),
        )
    }
}

/** One ticket inside a landing: its title, and what it says about itself in amber when it waits on the owner. */
@Composable
private fun PlanLandingTicketRow(row: PlanViewTicket, onClick: () -> Unit) {
    val amber = glanceColor(GlancePalette.amber)
    // "Needs you" is the ticket's note, said once, in amber; a ticket simply aboard says nothing (ov-522).
    val standing = row.note.orEmpty()
    // Its notes lead with that "Needs you" while it waits on the owner.
    val warnings = row.notes.drop(if (row.waitsOnOwner) 1 else 0)
    val attention = row.notes.any { it.attention }
    val spoken = row.accessibility.ifEmpty { listOfNotNull(row.heading, standing.ifEmpty { null }).joinToString(", ") }
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 4.dp).testTag("plan-landing-ticket-${row.key}")
            .semantics(mergeDescendants = true) { contentDescription = spoken },
    ) {
        Column(Modifier.weight(1f)) {
            Text(row.heading, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (standing.isNotEmpty()) {
                Text(
                    standing, style = MaterialTheme.typography.bodySmall,
                    color = if (row.waitsOnOwner) amber else MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-landing-ticket-${row.key}-note"),
                )
            }
            warnings.forEach { Text(it.text, style = MaterialTheme.typography.labelMedium, color = if (it.attention) amber else MaterialTheme.colorScheme.onSurfaceVariant) }
        }
        if (attention) {
            Spacer(Modifier.width(8.dp))
            Box(Modifier.size(6.dp).clip(CircleShape).background(amber))
        }
    }
}
