package com.farcooler.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
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
import com.farcooler.model.RulingWords
import com.farcooler.model.settledRulings
import com.farcooler.model.standingRulings
import kotlinx.coroutines.delay

// Decided for you (ov-304) in the Plan view: the calls the orchestrator made
// on the owner's behalf, after the plan's other sections. Standing rulings
// first, newest first, each with its short id, decision, why and what
// reversing costs; then the confirmed and reversed ones, a line each.
//
// No edit: the orchestrator is the only writer. Copy reference puts "ruling
// R-12: <decision>" on the clipboard for telling it. Nothing shows on a
// runner without `board_rulings`, or on a board with no rulings.

/** How the Plan view copies a ruling's reference. Null on a runner without `board_rulings`. */
class RulingsHook(val copy: (String) -> Unit)

/** The Decided for you rows, when there's anything to show. */
fun LazyListScope.rulingItems(plan: Plan, hook: RulingsHook?) {
    if (hook == null || plan.rulings.isEmpty()) return
    val standing = plan.standingRulings
    item(key = "plan/rulings") { PlanHeader(RulingWords.TITLE, standing.size, Modifier.testTag("plan-rulings")) }
    for (ruling in standing) {
        item(key = "plan/ruling/${ruling.id}") { PlanRulingRow(ruling, hook.copy, Modifier.animateItem()) }
    }
    for (ruling in plan.settledRulings) {
        item(key = "plan/ruling/${ruling.id}") { PlanSettledRulingRow(ruling, hook.copy, Modifier.animateItem()) }
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

/** A standing ruling: its short id and decision, why, what reversing costs and what it touches. */
@Composable
fun PlanRulingRow(ruling: PlanRuling, copy: (String) -> Unit, modifier: Modifier = Modifier) {
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
        trailingContent = { CopyReferenceButton(ruling, copy) },
        modifier = modifier.testTag("plan-ruling-${ruling.short}").rulingSemantics(ruling, copy),
    )
}

/** A confirmed or reversed ruling: one quiet line, with its note. */
@Composable
fun PlanSettledRulingRow(ruling: PlanRuling, copy: (String) -> Unit, modifier: Modifier = Modifier) {
    ListItem(
        leadingContent = { ShortId(ruling) },
        headlineContent = {
            Text(
                "${RulingWords.state(ruling.state)} · ${ruling.decision}",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        },
        supportingContent = if (ruling.note.isEmpty()) null else {
            { Text(ruling.note, maxLines = 2, overflow = TextOverflow.Ellipsis, color = MaterialTheme.colorScheme.outline) }
        },
        modifier = modifier.testTag("plan-ruling-${ruling.short}").rulingSemantics(ruling, copy),
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

/** One spoken label for the row, and Copy reference as its custom action. */
private fun Modifier.rulingSemantics(ruling: PlanRuling, copy: (String) -> Unit): Modifier =
    semantics(mergeDescendants = true) {
        contentDescription = RulingWords.accessibility(ruling)
        customActions = listOf(CustomAccessibilityAction(RulingWords.COPY_REFERENCE) { copy(ruling.reference); true })
    }
