package com.farcooler.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import com.farcooler.model.Plan
import com.farcooler.model.PlanRuling
import com.farcooler.model.RulingActions
import com.farcooler.model.RulingWords
import com.farcooler.model.openRulings
import com.farcooler.model.pastRulings
import kotlinx.coroutines.delay

// Decided for you (ov-304) in the Plan view: the calls the orchestrator made
// on the owner's behalf, after the plan's other sections. Only the open ones,
// newest first (ov-333), each with its short id, decision, why and what
// reversing costs. The owner acts on each: Keep, Reverse and Discuss from the
// row's menu, and Keep all in the section's header. The kept and reversed ones
// fold into Past decisions, a line each, closed until opened.
//
// Keep is the owner's own mark and never reaches the orchestrator. Reverse
// sends it the ruling's recorded reversal; Discuss quotes the ruling into its
// composer, unsent ([com.farcooler.model.RulingActions]). No count anywhere
// but the header's, and no notification. Nothing shows on a runner without
// `board_rulings`, or on a board with no rulings.

/**
 * What the Plan view does with rulings. Null on a runner without
 * `board_rulings`. [canMark] is `board_ruling_actions` (Keep, Keep all) and
 * [canAsk] is a running orchestrator to reverse or discuss with.
 */
class RulingsHook(
    val copy: (String) -> Unit,
    val canMark: Boolean = false,
    val canAsk: Boolean = false,
    val pastOpen: Boolean = false,
    val onTogglePast: () -> Unit = {},
    val keep: (PlanRuling) -> Unit = {},
    val keepAll: () -> Unit = {},
    val reverse: (PlanRuling) -> Unit = {},
    val discuss: (PlanRuling) -> Unit = {},
    /** What the last Reverse or Discuss had to say, under the open rulings. */
    val notice: String? = null,
)

/** The Decided for you and Past decisions rows, when there's anything to show. */
fun LazyListScope.rulingItems(plan: Plan, hook: RulingsHook?) {
    if (hook == null || plan.rulings.isEmpty()) return
    val open = plan.openRulings
    val past = plan.pastRulings
    if (open.isNotEmpty()) {
        item(key = "plan/rulings") {
            Row(verticalAlignment = Alignment.CenterVertically) {
                PlanHeader(RulingWords.TITLE, open.size, Modifier.weight(1f).testTag("plan-rulings"))
                if (hook.canMark && open.size > 1) {
                    TextButton(onClick = hook.keepAll, modifier = Modifier.testTag("plan-rulings-keep-all")) {
                        Text(RulingWords.KEEP_ALL)
                    }
                }
            }
        }
        for (ruling in open) {
            item(key = "plan/ruling/${ruling.id}") { PlanRulingRow(ruling, hook, Modifier.animateItem()) }
        }
        if (hook.notice != null) {
            item(key = "plan/rulings/notice") {
                Text(
                    hook.notice,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp).testTag("plan-rulings-notice"),
                )
            }
        }
    }
    if (past.isNotEmpty()) {
        item(key = "plan/rulings/past") {
            PlanHeader(
                RulingWords.PAST_DECISIONS, past.size, Modifier.testTag("plan-rulings-past-header"),
                open = hook.pastOpen, onClick = hook.onTogglePast,
            )
        }
        if (hook.pastOpen) {
            for (ruling in past) {
                item(key = "plan/ruling/${ruling.id}") { PlanSettledRulingRow(ruling, hook.copy, Modifier.animateItem()) }
            }
        }
    }
}

/** Copy reference: an icon button, a check while it's just copied. */
@Composable
private fun CopyReferenceButton(ruling: PlanRuling, copy: (String) -> Unit) {
    var copied by remember { mutableStateOf(false) }
    LaunchedEffect(copied) {
        if (copied) {
            delay(1500)
            copied = false
        }
    }
    IconButton(
        onClick = {
            copy(ruling.reference)
            copied = true
        },
        modifier = Modifier.testTag("plan-ruling-${ruling.short}-copy"),
    ) {
        Icon(
            if (copied) Icons.Outlined.Check else Icons.Outlined.ContentCopy,
            contentDescription = if (copied) RulingWords.COPIED else RulingWords.COPY_REFERENCE,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * An open ruling: its short id and decision, why, what reversing costs and what it
 * touches. Keep is one tap; Reverse, Discuss and Copy reference are in its menu,
 * and TalkBack has them as custom actions.
 */
@Composable
fun PlanRulingRow(ruling: PlanRuling, hook: RulingsHook, modifier: Modifier = Modifier) {
    // Reverse asks first (ruling R-18): it sends the orchestrator off to change
    // things. Keep and Discuss don't.
    var confirming by remember { mutableStateOf(false) }
    if (confirming) {
        AlertDialog(
            onDismissRequest = { confirming = false },
            title = { Text(RulingActions.confirmTitle(ruling)) },
            text = { Text(RulingActions.confirmMessage(ruling)) },
            confirmButton = {
                TextButton(onClick = { confirming = false; hook.reverse(ruling) }, modifier = Modifier.testTag("plan-ruling-${ruling.short}-reverse-confirm")) {
                    Text(RulingWords.REVERSE)
                }
            },
            dismissButton = { TextButton(onClick = { confirming = false }) { Text("Cancel") } },
            modifier = Modifier.testTag("plan-ruling-${ruling.short}-confirm"),
        )
    }
    ListItem(
        leadingContent = { ShortId(ruling) },
        headlineContent = { Text(ruling.decision, style = MaterialTheme.typography.titleSmall) },
        supportingContent = {
            Column {
                Labeled(RulingWords.WHY, ruling.why)
                Labeled(RulingWords.REVERSAL, ruling.reversal)
                RulingWords.touches(ruling)?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.outline, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        },
        trailingContent = { RulingActionsMenu(ruling, hook, onReverse = { confirming = true }) },
        modifier = modifier.testTag("plan-ruling-${ruling.short}").rulingSemantics(ruling, hook, onReverse = { confirming = true }),
    )
}

/** Keep, and a menu with Reverse, Discuss and Copy reference. Reverse and Discuss are off, never hidden, with no orchestrator. */
@Composable
private fun RulingActionsMenu(ruling: PlanRuling, hook: RulingsHook, onReverse: () -> Unit) {
    var open by remember { mutableStateOf(false) }
    Row(verticalAlignment = Alignment.CenterVertically) {
        if (hook.canMark) {
            IconButton(onClick = { hook.keep(ruling) }, modifier = Modifier.testTag("plan-ruling-${ruling.short}-keep")) {
                Icon(Icons.Outlined.Check, contentDescription = RulingWords.KEEP, tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        Box {
            IconButton(onClick = { open = true }, modifier = Modifier.testTag("plan-ruling-${ruling.short}-more")) {
                Icon(Icons.Outlined.MoreVert, contentDescription = "More for ${ruling.short}", tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                if (hook.canMark) {
                    DropdownMenuItem(
                        text = { Text(RulingWords.REVERSE) },
                        enabled = hook.canAsk,
                        onClick = { open = false; onReverse() },
                        modifier = Modifier.testTag("plan-ruling-${ruling.short}-reverse"),
                    )
                    DropdownMenuItem(
                        text = { Text(RulingWords.DISCUSS) },
                        enabled = hook.canAsk,
                        onClick = { open = false; hook.discuss(ruling) },
                        modifier = Modifier.testTag("plan-ruling-${ruling.short}-discuss"),
                    )
                }
                DropdownMenuItem(
                    text = { Text(RulingWords.COPY_REFERENCE) },
                    onClick = { open = false; hook.copy(ruling.reference) },
                    modifier = Modifier.testTag("plan-ruling-${ruling.short}-copy"),
                )
            }
        }
    }
}

/** A kept or reversed ruling: one quiet line, with its note. */
@Composable
fun PlanSettledRulingRow(ruling: PlanRuling, copy: (String) -> Unit, modifier: Modifier = Modifier) {
    ListItem(
        leadingContent = { ShortId(ruling) },
        headlineContent = {
            Text(
                "${RulingWords.settled(ruling)} · ${ruling.decision}",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        },
        supportingContent = if (ruling.note.isEmpty()) null else {
            { Text(ruling.note, maxLines = 2, overflow = TextOverflow.Ellipsis, color = MaterialTheme.colorScheme.outline) }
        },
        // A kept ruling can still be reversed, so it can be cited too (review 1005a F4).
        trailingContent = { CopyReferenceButton(ruling, copy) },
        modifier = modifier.testTag("plan-ruling-${ruling.short}").rulingSemantics(ruling, RulingsHook(copy)),
    )
}

@Composable
private fun ShortId(ruling: PlanRuling) {
    Box(Modifier.width(40.dp), contentAlignment = Alignment.CenterStart) {
        Text(
            ruling.short,
            style = MaterialTheme.typography.labelLarge.copy(fontFeatureSettings = "tnum"),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
        )
    }
}

@Composable
private fun Labeled(label: String, text: String) {
    Text(
        buildAnnotatedString {
            withStyle(SpanStyle(fontWeight = FontWeight.Medium)) { append("$label: ") }
            append(text)
        },
        style = MaterialTheme.typography.bodyMedium,
    )
}

/** One spoken label for the row, and its actions as TalkBack's custom actions. */
private fun Modifier.rulingSemantics(ruling: PlanRuling, hook: RulingsHook, onReverse: () -> Unit = {}): Modifier =
    semantics(mergeDescendants = true) {
        contentDescription = RulingWords.accessibility(ruling)
        val actions = mutableListOf<CustomAccessibilityAction>()
        if (hook.canMark && ruling.isStanding) {
            actions += CustomAccessibilityAction(RulingWords.KEEP) { hook.keep(ruling); true }
            if (hook.canAsk) {
                actions += CustomAccessibilityAction(RulingWords.REVERSE) { onReverse(); true }
                actions += CustomAccessibilityAction(RulingWords.DISCUSS) { hook.discuss(ruling); true }
            }
        }
        actions += CustomAccessibilityAction(RulingWords.COPY_REFERENCE) { hook.copy(ruling.reference); true }
        customActions = actions
    }
