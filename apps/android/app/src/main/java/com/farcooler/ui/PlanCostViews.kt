package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.farcooler.model.GlancePalette
import com.farcooler.model.PlanBudget
import com.farcooler.model.PlanCostRead
import com.farcooler.model.PlanCostWords
import com.farcooler.model.PlanTheme
import com.farcooler.model.PlanTrend

// Cost on the plan, drawn on Android (ov-307), as on the Mac and the iPhone.
// The words and rules are `model/PlanCost.kt`'s; these lay them out. Color is
// for the one state that needs attention, a budget gone over, and it always
// comes with a glyph and words. Every block is one TalkBack element that says
// its numbers in words.

/** A theme's seven days as bars, oldest first and today last, each against the busiest day. Neutral: the trend is a shape to read, not a state. */
@Composable
fun PlanTrendBars(trend: PlanTrend, modifier: Modifier = Modifier, height: Int = 28) {
    val scheme = MaterialTheme.colorScheme
    val heights = trend.heights()
    Row(
        modifier
            .fillMaxWidth()
            .height(height.dp)
            .semantics(mergeDescendants = true) { contentDescription = PlanCostWords.trendSpoken(trend) }
            .testTag("plan-trend"),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.Bottom,
    ) {
        heights.forEachIndexed { index, share ->
            Box(Modifier.weight(1f).fillMaxHeight(), contentAlignment = Alignment.BottomCenter) {
                // The baseline, so a day with nothing still has a place.
                Box(Modifier.fillMaxWidth().height(2.dp).clip(CircleShape).background(scheme.surfaceContainerHighest))
                if (share > 0) {
                    // Today, the last bar, is the darkest: the one that's still moving.
                    Box(
                        Modifier.fillMaxWidth().height(maxOf(2.0, height * share).dp)
                            .background(if (index == heights.lastIndex) scheme.onSurfaceVariant else scheme.outline),
                    )
                }
            }
        }
    }
}

/** "Over budget: 6.1M of 5M tokens" in amber with a glyph, or "1.2M of 5M tokens budgeted" in the neutral secondary. */
@Composable
fun PlanBudgetLine(budget: PlanBudget, modifier: Modifier = Modifier) {
    val amber = glanceColor(GlancePalette.amber)
    Row(
        modifier.semantics(mergeDescendants = true) { contentDescription = PlanCostWords.budgetSpoken(budget) }
            .testTag(if (budget.isOver) "plan-budget-over" else "plan-budget"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (budget.isOver) {
            Icon(Icons.Outlined.ErrorOutline, contentDescription = null, modifier = Modifier.size(16.dp), tint = amber)
            Spacer(Modifier.width(6.dp))
        }
        Text(
            PlanCostWords.budgetLine(budget),
            style = MaterialTheme.typography.bodySmall.copy(fontWeight = if (budget.isOver) FontWeight.Medium else FontWeight.Normal),
            color = if (budget.isOver) amber else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** Spend, a budget or a trend: a theme with none has nothing to say. */
fun planThemeHasSpend(theme: PlanTheme): Boolean =
    (theme.spend?.totalTokens ?: 0L) > 0 || theme.budgetTokens != null || PlanCostWords.trend(theme.trendTokens) != null

/** A theme's spend: what its lanes spent, the budget against it, and the last seven days. Draw it only when [planThemeHasSpend]. */
@Composable
fun PlanThemeSpend(theme: PlanTheme, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth().testTag("plan-theme-spend"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        theme.spend?.takeIf { it.totalTokens > 0 }?.let {
            Text(com.farcooler.model.PlanWords.spend(it), style = MaterialTheme.typography.bodyLarge)
            if ((it.costMicros ?: 0) > 0) {
                Text(com.farcooler.model.TaskUsageFormat.API_EQUIVALENT, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        PlanCostWords.budget(theme.spend, theme.budgetTokens)?.let { PlanBudgetLine(it) }
        PlanCostWords.trend(theme.trendTokens)?.let { trend ->
            Column(
                verticalArrangement = Arrangement.spacedBy(4.dp),
                modifier = Modifier.semantics(mergeDescendants = true) { contentDescription = PlanCostWords.trendSpoken(trend) },
            ) {
                PlanTrendBars(trend)
                Text("Last 7 days by UTC day, today on the right", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

/**
 * The runner's week, and cost per finished card by harness and model. The week
 * has no percentage, and says why; a pair with fewer than three finished cards
 * isn't drawn, and the note says how many weren't.
 */
@Composable
fun PlanCostBlock(cost: PlanCostRead, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp).testTag("plan-cost"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        if (cost.weekTokens > 0) {
            Column(
                verticalArrangement = Arrangement.spacedBy(4.dp),
                modifier = Modifier.semantics(mergeDescendants = true) {}.testTag("plan-cost-week"),
            ) {
                Text(PlanCostWords.week(cost.weekTokens), style = MaterialTheme.typography.bodyLarge)
                Text(PlanCostWords.WEEK_NOTE, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        if (cost.compare.isNotEmpty() || cost.compareHeldBack > 0 || cost.inFlightTokens > 0) {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    "Cost per landed card",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.semantics { heading() },
                )
                for (row in cost.compare.map { PlanCostWords.compareRow(it) }) {
                    Column(
                        verticalArrangement = Arrangement.spacedBy(2.dp),
                        modifier = Modifier.semantics(mergeDescendants = true) { contentDescription = row.spoken }.testTag("plan-compare-${row.id}"),
                    ) {
                        Text(row.title, style = MaterialTheme.typography.titleSmall)
                        Text(row.detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                PlanCostWords.heldBack(cost.compareHeldBack)?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("plan-compare-held-back"))
                }
                PlanCostWords.inFlight(cost)?.let {
                    Column(
                        verticalArrangement = Arrangement.spacedBy(2.dp),
                        modifier = Modifier.semantics(mergeDescendants = true) {}.testTag("plan-cost-in-flight"),
                    ) {
                        Text("In flight", style = MaterialTheme.typography.titleSmall)
                        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                if (cost.compare.isNotEmpty()) {
                    Text(PlanCostWords.COMPARE_NOTE, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.outline)
                }
            }
        }
    }
}
