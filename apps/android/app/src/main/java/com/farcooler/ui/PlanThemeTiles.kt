package com.farcooler.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.IntrinsicSize.Min
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.Contrast
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.GlancePalette
import com.farcooler.model.PlanCardGlyph
import com.farcooler.model.PlanThemeGrid
import com.farcooler.model.PlanThemeTile

// The themes grid (ov-503, design 5), drawn for the phone: a tile a theme, modeled on the site's "What it's building
// now". Each tile says the theme's name, how much is done, a meter, up to three cards and, in amber, what waits on the
// owner. One column on a phone, two on a tablet. `PlanThemeGrid` decides which cards and which themes lead; a tile
// opens the theme's page, where the story now lives.

/** The grid as list items, [columns] to a row, every tile in a row as tall as the tallest. */
fun LazyListScope.themeGrid(grid: PlanThemeGrid, columns: Int, budget: (String) -> String?, onOpen: (String) -> Unit) {
    val cells = grid.tiles.map<PlanThemeTile, @Composable () -> Unit> { tile ->
        { PlanThemeTileCard(tile, budget(tile.id)) { onOpen(tile.id) } }
    } + listOfNotNull(
        grid.finishedTitle?.let { title -> { PlanFinishedTile(title, grid.finishedWords ?: "", grid.finished.map { it.name }) } },
    )
    cells.chunked(columns.coerceAtLeast(1)).forEachIndexed { index, row ->
        item(key = "plan/themes/row/$index") {
            Row(
                Modifier.fillMaxWidth().height(IntrinsicSize.Min).padding(horizontal = 16.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                row.forEach { cell -> Box(Modifier.weight(1f).fillMaxHeight()) { cell() } }
                // A short last row keeps its tiles the width of the rest.
                repeat(columns - row.size) { Spacer(Modifier.weight(1f)) }
            }
        }
    }
}

private fun glyph(glyph: PlanCardGlyph): ImageVector = when (glyph) {
    PlanCardGlyph.DONE -> Icons.Outlined.CheckCircleOutline
    PlanCardGlyph.IN_PROGRESS -> Icons.Outlined.Contrast
    PlanCardGlyph.NOT_STARTED -> Icons.Outlined.RadioButtonUnchecked
}

private fun word(glyph: PlanCardGlyph): String = when (glyph) {
    PlanCardGlyph.DONE -> "Done"
    PlanCardGlyph.IN_PROGRESS -> "In progress"
    PlanCardGlyph.NOT_STARTED -> "Not started"
}

@Composable
fun PlanThemeTileCard(tile: PlanThemeTile, budget: String?, onClick: () -> Unit) {
    val amber = glanceColor(GlancePalette.amber)
    val spoken = listOfNotNull(
        tile.name, tile.progress,
        tile.cards.joinToString(", ") { "${it.title.ifEmpty { it.key }}, ${word(it.glyph)}" }.ifEmpty { null },
        budget, tile.ownerAsk?.let { "Needs you: $it" },
    ).joinToString(". ")
    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        modifier = Modifier.fillMaxWidth().fillMaxHeight().clip(MaterialTheme.shapes.large).clickable(role = Role.Button, onClick = onClick)
            .testTag("plan-theme-tile-${tile.name}").semantics(mergeDescendants = true) { contentDescription = spoken },
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(tile.name, style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold), maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(
                    tile.progress, style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
                    color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("plan-tile-${tile.name}-progress"),
                )
            }
            PlanMeter(tile.fraction.toFloat())
            tile.cards.forEach { card ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(
                        glyph(card.glyph), contentDescription = null, modifier = Modifier.size(16.dp),
                        tint = if (card.glyph == PlanCardGlyph.NOT_STARTED) MaterialTheme.colorScheme.outline else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        card.title.ifEmpty { card.key }, style = MaterialTheme.typography.bodySmall,
                        color = if (card.glyph == PlanCardGlyph.DONE) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
                        maxLines = 1, overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            if (budget != null) {
                Text(budget, style = MaterialTheme.typography.labelMedium.copy(fontWeight = FontWeight.Medium), color = amber, modifier = Modifier.testTag("plan-tile-${tile.name}-over-budget"))
            }
            tile.ownerAsk?.let { PlanAsk(it, maxLines = 3) }
        }
    }
}

/** How much is done: the filled part of a 4 dp bar, neutral. */
@Composable
fun PlanMeter(fraction: Float, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    Box(modifier.fillMaxWidth().height(4.dp).clip(CircleShape).background(scheme.surfaceContainerHighest)) {
        Box(Modifier.fillMaxHeight().fillMaxWidth(fraction.coerceIn(0f, 1f)).background(scheme.onSurfaceVariant))
    }
}

/** The one tile for what's finished: not a button, there's nothing to open. */
@Composable
fun PlanFinishedTile(title: String, words: String, names: List<String>) {
    Surface(
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surface,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        modifier = Modifier.fillMaxWidth().fillMaxHeight().testTag("plan-finished-tile")
            .semantics(mergeDescendants = true) { contentDescription = "$title, $words. ${names.joinToString(", ")}" },
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(title, style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold))
            Text(words, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(6.dp))
            Text(
                names.joinToString(" · "), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 6, overflow = TextOverflow.Ellipsis,
            )
        }
    }
}
