package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.outlined.Block
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.Contrast
import androidx.compose.material.icons.outlined.HighlightOff
import androidx.compose.material.icons.outlined.RadioButtonChecked
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import com.farcooler.model.BoardPage
import com.farcooler.model.GlancePalette
import com.farcooler.model.PageBlock
import com.farcooler.model.PageCell
import com.farcooler.model.PageColumn
import com.farcooler.model.PageDestination
import com.farcooler.model.PageDoc
import com.farcooler.model.PageEntry
import com.farcooler.model.PageItem
import com.farcooler.model.PageLayout
import com.farcooler.model.PageLinks
import com.farcooler.model.PageMarkdown
import com.farcooler.model.PageRef
import com.farcooler.model.PageResolved
import com.farcooler.model.PageSpeech
import com.farcooler.model.PageState
import com.farcooler.model.PageStep
import com.farcooler.model.PageTone
import com.farcooler.model.PageWords
import com.farcooler.model.PageWorld
import com.farcooler.model.TaskKeyLinks

// An orchestrator's page, drawn natively in Compose (ov-269 design 3, 6.5;
// ov-285): the nine blocks in Material's own type and sentence case. The rules
// are `model/Page.kt`'s and `model/PageLive.kt`'s, the twins of AgentKit's, so
// a page reads the same on the Mac, the iPhone and here.
//
// Color is for attention alone: amber on a block or cell the orchestrator
// marked `attention`, and on a question still waiting on the owner. Every
// state has its word beside its glyph. Navigation is the only action: a
// reference goes to [onOpen]; a web link opens in the browser, and only after
// it's checked again as `https`.

/** A page whole: its title, when it was written and by whom, then its blocks. */
@Composable
fun PageView(page: BoardPage, world: PageWorld, onOpen: (PageDestination) -> Unit, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth().testTag("page-${page.slot}"), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(page.doc?.title ?: page.title, style = MaterialTheme.typography.titleLarge, modifier = Modifier.semantics { heading() })
            Text(
                (listOf(PageWords.updated(page, world.nowMs)) + listOfNotNull(page.actor.ifEmpty { null })).joinToString(" · "),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.testTag("page-updated"),
            )
        }
        val doc = page.doc
        if (doc != null) PageBlocks(doc, world, onOpen)
        else Text(PageWords.UNREADABLE, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("page-unreadable"))
    }
}

/** A document's blocks, top to bottom: what a page draws under its title, and what a theme's page draws for a page anchored to it. */
@Composable
fun PageBlocks(doc: PageDoc, world: PageWorld, onOpen: (PageDestination) -> Unit, modifier: Modifier = Modifier) {
    val uri = LocalUriHandler.current
    // The one place a destination is opened: a web link to the browser, after
    // its check again, and everything else to the app.
    val open: (PageDestination) -> Unit = { destination ->
        if (destination is PageDestination.Url) {
            if (PageLinks.https(destination.url) != null) runCatching { uri.openUri(destination.url) }
        } else {
            onOpen(destination)
        }
    }
    BoxWithConstraints(modifier.fillMaxWidth()) {
        val width = maxWidth.value.toInt()
        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            doc.blocks.forEachIndexed { index, block ->
                // A heading opens a new section, so it sits further from what's above.
                val gap = if (block is PageBlock.Heading && index > 0) 8.dp else 0.dp
                Box(Modifier.padding(top = gap)) { PageBlockView(block, world, width, open) }
            }
        }
    }
}

@Composable
private fun PageBlockView(block: PageBlock, world: PageWorld, width: Int, open: (PageDestination) -> Unit) {
    when (block) {
        is PageBlock.Heading -> Text(
            block.text,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.semantics { heading() },
        )
        is PageBlock.Text -> PageText(block.md, block.tone, open)
        is PageBlock.Stats -> PageStats(block)
        is PageBlock.Progress -> PageProgress(block)
        is PageBlock.Table ->
            if (PageLayout.stacks(block.columns.size, width)) PageStackedTable(block.columns, block.rows, world, open)
            else PageGridTable(block.columns, block.rows, world, open)
        is PageBlock.ListBlock -> PageList(block.items, world, open)
        is PageBlock.Timeline -> PageTimeline(PageLayout.ordered(block.entries, block.given), world, open)
        is PageBlock.Steps -> PageSteps(block.steps, down = PageLayout.stepsDown(width))
        is PageBlock.Links -> PageLinksRow(block.refs, world, open)
        is PageBlock.Unknown -> Text(
            block.alt ?: PageWords.NEWER_BLOCK,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.testTag("page-block-unknown"),
        )
    }
}

@Composable
private fun amber(): Color = glanceColor(GlancePalette.amber)

// MARK: - Text

/** A text block: the Markdown subset, amber when the orchestrator marked it. */
@Composable
private fun PageText(md: String, tone: PageTone, open: (PageDestination) -> Unit) {
    val color = if (tone == PageTone.ATTENTION) amber() else MaterialTheme.colorScheme.onSurface
    val linker = LocalTaskKeyLinker.current
    val link = MaterialTheme.colorScheme.primary
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    val code = MaterialTheme.colorScheme.surfaceContainerHighest
    fun line(text: String) = pageInline(text, linker, link, muted, code, open)
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        for (piece in PageMarkdown.pieces(md)) {
            when (piece) {
                is PageMarkdown.Piece.Prose -> Text(line(piece.text), style = MaterialTheme.typography.bodyLarge, color = color)
                is PageMarkdown.Piece.Item -> Row(Modifier.padding(start = (piece.depth * 16).dp)) {
                    Text(piece.marker, style = MaterialTheme.typography.bodyLarge, color = muted, modifier = Modifier.widthIn(min = 16.dp))
                    Spacer(Modifier.width(6.dp))
                    Text(line(piece.text), style = MaterialTheme.typography.bodyLarge, color = color)
                }
                is PageMarkdown.Piece.Plain -> Text(piece.text, style = MaterialTheme.typography.bodyLarge, color = color)
            }
        }
    }
}

/** A line's runs as one styled string: `https` links that open after a check, each with its domain after it, and task keys. */
private fun pageInline(
    text: String,
    linker: TaskKeyLinker,
    linkColor: Color,
    muted: Color,
    codeBackground: Color,
    open: (PageDestination) -> Unit,
): AnnotatedString = buildAnnotatedString {
    val linkStyle = TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline))
    val quoted = mutableListOf<IntRange>()
    for (run in PageMarkdown.inline(text)) {
        val span = run.span
        val start = length
        val style = SpanStyle(
            fontWeight = if (span.bold) FontWeight.Bold else null,
            fontStyle = if (span.italic) FontStyle.Italic else null,
            fontFamily = if (span.code) FontFamily.Monospace else null,
            background = if (span.code) codeBackground else Color.Unspecified,
            color = if (run.domain) muted else Color.Unspecified,
        )
        val url = span.link
        withStyle(style) { append(span.text) }
        if (url != null && length > start) addLink(LinkAnnotation.Clickable(url, linkStyle) { open(PageDestination.Url(url)) }, start, length)
        if ((span.code || url != null) && length > start) quoted.add(start until length)
    }
    if (linker.index.isEmpty) return@buildAnnotatedString
    for (match in TaskKeyLinks.matches(toString(), linker.index)) {
        if (quoted.any { it.first < match.end && match.start <= it.last }) continue
        val url = TaskKeyLinks.url(linker.index.runner, match.key)
        addLink(LinkAnnotation.Clickable(url, linkStyle) { linker.follow(url) }, match.start, match.end)
    }
}

// MARK: - Stats and progress

/** A row of figures that wraps. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun PageStats(block: PageBlock.Stats) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(28.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        for (stat in block.items) {
            Column(Modifier.semantics(mergeDescendants = true) {}) {
                Text(stat.label, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(
                    stat.value,
                    style = MaterialTheme.typography.headlineSmall.copy(fontFeatureSettings = "tnum"),
                    color = if (stat.tone == PageTone.ATTENTION) amber() else MaterialTheme.colorScheme.onSurface,
                )
                stat.detail?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
            }
        }
    }
}

/** A bar in neutral fills, with "3 of 10" in words. */
@Composable
private fun PageProgress(block: PageBlock.Progress) {
    val scheme = MaterialTheme.colorScheme
    Column(
        verticalArrangement = Arrangement.spacedBy(6.dp),
        modifier = Modifier.semantics(mergeDescendants = true) {
            contentDescription = block.label
            stateDescription = PageSpeech.progress(block)
        },
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(block.label, style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
            Text(
                PageWords.progress(block.done, block.total),
                style = MaterialTheme.typography.labelLarge.copy(fontFeatureSettings = "tnum"),
                color = scheme.onSurfaceVariant,
            )
        }
        // Done darkest, then each part in a lighter fill; without parts, done alone.
        val fills = listOf(scheme.onSurfaceVariant, scheme.outline, scheme.outlineVariant, scheme.surfaceVariant)
        val shares = if (block.parts.isEmpty()) listOf(PageLayout.fraction(block.done, block.total))
        else {
            var left = 1f
            block.parts.take(4).map { part -> minOf(left, PageLayout.fraction(part.count, block.total)).also { left -= it } }
        }
        Row(
            Modifier.fillMaxWidth().height(6.dp).clip(CircleShape).background(scheme.surfaceContainerHighest),
            horizontalArrangement = Arrangement.spacedBy(1.dp),
        ) {
            shares.forEachIndexed { index, share ->
                if (share > 0f) Box(Modifier.weight(share).fillMaxHeight().background(fills[index]))
            }
            val rest = 1f - shares.sum()
            if (rest > 0.0001f) Box(Modifier.weight(rest))
        }
        val words = listOfNotNull(block.detail) + block.parts.map { "${it.label} ${it.count}" }
        if (words.isNotEmpty()) Text(words.joinToString(" · "), style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant)
    }
}

// MARK: - Tables

/** One cell's words: a link in the primary color with its domain after it when it leaves the app, or the orchestrator's words. */
@Composable
private fun PageCellText(cell: PageCell, world: PageWorld, open: (PageDestination) -> Unit, modifier: Modifier = Modifier, strong: Boolean = false) {
    val parts = world.parts(cell)
    val color = when {
        parts.destination != null -> MaterialTheme.colorScheme.primary
        cell.tone == PageTone.ATTENTION -> amber()
        else -> MaterialTheme.colorScheme.onSurface
    }
    val style = MaterialTheme.typography.bodyMedium.copy(
        fontFamily = if (cell.mono) FontFamily.Monospace else null,
        fontWeight = if (strong || cell.tone == PageTone.ATTENTION) FontWeight.Medium else null,
    )
    val destination = parts.destination
    Text(
        buildAnnotatedString {
            append(parts.text)
            parts.domain?.let { withStyle(SpanStyle(color = MaterialTheme.colorScheme.onSurfaceVariant)) { append(" $it") } }
        },
        style = style,
        color = color,
        modifier = if (destination != null) modifier.clickable(role = Role.Button) { open(destination) } else modifier,
    )
}

/** What a row's links are, as TalkBack actions: one "Open …" per link, every column. */
private fun rowActions(cells: List<PageCell>, world: PageWorld, open: (PageDestination) -> Unit) =
    world.actions(cells).map { (name, destination) -> CustomAccessibilityAction(name) { open(destination); true } }

/** A grid on a wide surface: columns weighted, the growing one wider. */
@Composable
private fun PageGridTable(columns: List<PageColumn>, rows: List<List<PageCell>>, world: PageWorld, open: (PageDestination) -> Unit) {
    fun weight(c: PageColumn) = if (c.grow) 2f else 1f
    fun align(c: PageColumn) = when (c.align) {
        PageColumn.Align.START -> Alignment.Start
        PageColumn.Align.CENTER -> Alignment.CenterHorizontally
        PageColumn.Align.END -> Alignment.End
    }
    Column {
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp)) {
            for (column in columns) {
                Column(Modifier.weight(weight(column)), horizontalAlignment = align(column)) {
                    Text(column.title, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
        rows.forEachIndexed { index, row ->
            Row(
                Modifier.fillMaxWidth()
                    .clip(RoundedCornerShape(Radius.small))
                    .background(if (index % 2 == 1) MaterialTheme.colorScheme.surfaceContainer else Color.Transparent)
                    .padding(horizontal = 8.dp, vertical = 6.dp)
                    .testTag("page-row-$index")
                    .semantics(mergeDescendants = true) {
                        contentDescription = PageLayout.spokenRow(columns, row, world)
                        customActions = rowActions(row, world, open)
                    },
            ) {
                columns.forEachIndexed { c, column ->
                    Column(Modifier.weight(weight(column)), horizontalAlignment = align(column)) {
                        row.getOrNull(c)?.let { PageCellText(it, world, open) }
                    }
                }
            }
        }
    }
}

/** Stacked rows on a narrow surface (design 3.7): the first column is the row's title, the others `title  value` under it. */
@Composable
private fun PageStackedTable(columns: List<PageColumn>, rows: List<List<PageCell>>, world: PageWorld, open: (PageDestination) -> Unit) {
    Column(Modifier.testTag("page-stacked-table")) {
        rows.forEachIndexed { index, row ->
            Column(
                Modifier.fillMaxWidth()
                    .clip(RoundedCornerShape(Radius.small))
                    .background(if (index % 2 == 1) MaterialTheme.colorScheme.surfaceContainer else Color.Transparent)
                    .padding(horizontal = 8.dp, vertical = 8.dp)
                    .testTag("page-stacked-row-$index")
                    .semantics(mergeDescendants = true) {
                        contentDescription = PageLayout.spokenRow(columns, row, world)
                        customActions = rowActions(row, world, open)
                    },
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                row.firstOrNull()?.let { PageCellText(it, world, open, strong = true) }
                columns.zip(row).drop(1).forEach { (column, cell) ->
                    if (world.cellText(cell).isNotEmpty()) {
                        Row(Modifier.padding(start = 12.dp), verticalAlignment = Alignment.Top) {
                            Text(
                                column.title,
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.widthIn(min = 72.dp),
                            )
                            Spacer(Modifier.width(12.dp))
                            PageCellText(cell, world, open)
                        }
                    }
                }
            }
        }
    }
}

// MARK: - List, timeline, steps, links

/** A state's glyph: a shape to scan by, never the only signal. */
private fun glyph(state: PageState): ImageVector? = when (state) {
    PageState.DONE -> Icons.Outlined.CheckCircleOutline
    PageState.ACTIVE -> Icons.Outlined.RadioButtonChecked
    PageState.WAITING -> Icons.Outlined.Contrast
    PageState.BLOCKED -> Icons.Outlined.Block
    PageState.FAILED -> Icons.Outlined.HighlightOff
    PageState.TODO -> Icons.Outlined.RadioButtonUnchecked
    PageState.NONE -> null
}

@Composable
private fun StateGlyph(state: PageState, tone: PageTone) {
    Box(Modifier.width(24.dp), contentAlignment = Alignment.CenterStart) {
        glyph(state)?.let {
            Icon(
                it, contentDescription = null, modifier = Modifier.size(18.dp),
                tint = if (tone == PageTone.ATTENTION) amber() else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/** The trailing words of a row with a live reference: a question's "Needs you" in amber, a card's status, a link's domain. */
@Composable
private fun RefTrailer(resolved: PageResolved) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        // A web link with no label of its own has its domain as its name: the
        // row's words are the orchestrator's, so the domain is said here.
        (resolved.status ?: resolved.name.takeIf { resolved.destination?.isExternal == true })?.let {
            Text(
                it,
                style = MaterialTheme.typography.labelLarge,
                fontWeight = if (resolved.statusTone == PageTone.ATTENTION) FontWeight.Medium else null,
                color = if (resolved.statusTone == PageTone.ATTENTION) amber() else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        resolved.destination?.let {
            Icon(
                if (it.isExternal) Icons.AutoMirrored.Outlined.OpenInNew else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                contentDescription = null,
                modifier = Modifier.padding(start = 4.dp).size(16.dp),
                tint = MaterialTheme.colorScheme.outline,
            )
        }
    }
}

/** A row that opens its destination when it has one, and speaks as one. */
@Composable
private fun PageRow(destination: PageDestination?, shaded: Boolean, spoken: String, tag: String, open: (PageDestination) -> Unit, content: @Composable () -> Unit) {
    Box(
        Modifier.fillMaxWidth()
            .heightIn(min = 40.dp)
            .clip(RoundedCornerShape(Radius.small))
            .background(if (shaded) MaterialTheme.colorScheme.surfaceContainer else Color.Transparent)
            .then(if (destination != null) Modifier.clickable(role = Role.Button) { open(destination) } else Modifier)
            .padding(horizontal = 8.dp, vertical = 8.dp)
            .testTag(tag)
            .semantics(mergeDescendants = true) { contentDescription = spoken },
    ) { content() }
}

/** Rows with a state glyph and its word, read-only: a checklist, a risk list, a "waiting on" list. */
@Composable
private fun PageList(items: List<PageItem>, world: PageWorld, open: (PageDestination) -> Unit) {
    Column {
        items.forEachIndexed { index, item ->
            val resolved = item.ref?.let(world::resolve)
            PageRow(resolved?.destination, index % 2 == 1, PageSpeech.item(item, resolved), "page-item-$index", open) {
                Row(verticalAlignment = Alignment.Top) {
                    StateGlyph(item.state, item.tone)
                    Column(Modifier.weight(1f)) {
                        Text(
                            item.text,
                            style = MaterialTheme.typography.bodyLarge,
                            color = if (item.tone == PageTone.ATTENTION) amber() else MaterialTheme.colorScheme.onSurface,
                        )
                        item.detail?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                    }
                    Spacer(Modifier.width(8.dp))
                    Column(horizontalAlignment = Alignment.End) {
                        item.state.word?.let { Text(it, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                        resolved?.let { RefTrailer(it) }
                    }
                }
            }
        }
    }
}

/** When each thing happened, in the viewer's zone. */
@Composable
private fun PageTimeline(entries: List<PageEntry>, world: PageWorld, open: (PageDestination) -> Unit) {
    Column {
        entries.forEachIndexed { index, entry ->
            val resolved = entry.ref?.let(world::resolve)
            val time = timelineTime(entry.at, world.nowMs)
            PageRow(resolved?.destination, false, listOfNotNull(time, entry.text, resolved?.spoken).joinToString(", "), "page-entry-$index", open) {
                Row(verticalAlignment = Alignment.Top) {
                    Text(
                        time,
                        style = MaterialTheme.typography.labelLarge.copy(fontFeatureSettings = "tnum"),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.width(88.dp),
                    )
                    Text(entry.text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                    resolved?.let {
                        Spacer(Modifier.width(8.dp))
                        RefTrailer(it)
                    }
                }
            }
        }
    }
}

/** A pipeline of chips: across, wrapping, on a wide surface; down the page on a narrow one. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun PageSteps(steps: List<PageStep>, down: Boolean) {
    if (down) {
        Column(Modifier.testTag("page-steps"), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            for (step in steps) StepChip(step)
        }
    } else {
        FlowRow(Modifier.testTag("page-steps"), horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            steps.forEachIndexed { index, step ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (index > 0) {
                        Icon(
                            Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null,
                            modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.outline,
                        )
                    }
                    StepChip(step)
                }
            }
        }
    }
}

@Composable
private fun StepChip(step: PageStep) {
    Surface(
        shape = CircleShape,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier.semantics(mergeDescendants = true) { contentDescription = PageSpeech.step(step) },
    ) {
        Row(Modifier.padding(start = 8.dp, end = 12.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            StateGlyph(step.state, PageTone.NEUTRAL)
            Text(
                step.label,
                style = MaterialTheme.typography.labelLarge,
                fontWeight = if (step.state == PageState.ACTIVE) FontWeight.SemiBold else null,
            )
        }
    }
}

/** A row of link chips: an app destination, or an `https` page with its domain beside its label. One that resolves to nothing is plain words. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun PageLinksRow(refs: List<PageRef>, world: PageWorld, open: (PageDestination) -> Unit) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        refs.forEachIndexed { index, ref ->
            val resolved = world.resolve(ref)
            val destination = resolved.destination
            if (destination == null) {
                Text(
                    resolved.name,
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(vertical = 8.dp).testTag("page-link-$index"),
                )
            } else {
                Surface(
                    onClick = { open(destination) },
                    shape = RoundedCornerShape(Radius.small),
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                    modifier = Modifier.heightIn(min = 40.dp).testTag("page-link-$index").semantics { contentDescription = resolved.spoken },
                ) {
                    Row(Modifier.padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(resolved.name, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
                        if (destination.isExternal) {
                            resolved.status?.let {
                                Spacer(Modifier.width(6.dp))
                                Text(it, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        }
                        Icon(
                            if (destination.isExternal) Icons.AutoMirrored.Outlined.OpenInNew else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                            contentDescription = null,
                            modifier = Modifier.padding(start = 4.dp).size(16.dp),
                            tint = MaterialTheme.colorScheme.outline,
                        )
                    }
                }
            }
        }
    }
}
