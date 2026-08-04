package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import com.farcooler.model.Markdown

/**
 * A rendered markdown reply.
 *
 * The parser is shared logic in [Markdown]; this is the drawing, which is each
 * platform's own job. Plain text here would mean a table arrives as a wall of
 * pipes and a heading as a line starting with a hash — the same conversation,
 * unreadable.
 */
@Composable
fun MarkdownText(text: String, secondary: Boolean = false, modifier: Modifier = Modifier) {
    val blocks = remember(text) { Markdown.blocks(text) }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        for (block in blocks) {
            when (block) {
                is Markdown.Block.Paragraph -> Body(block.text, secondary)

                is Markdown.Block.Heading -> Text(
                    inline(block.text),
                    style = when (block.level) {
                        1 -> MaterialTheme.typography.titleMedium
                        2 -> MaterialTheme.typography.titleSmall
                        else -> MaterialTheme.typography.bodyLarge
                    },
                    fontWeight = FontWeight.SemiBold,
                )

                is Markdown.Block.Bullet -> Marker("•", block.text, block.depth, secondary)

                is Markdown.Block.Numbered ->
                    Marker("${block.number}.", block.text, block.depth, secondary)

                is Markdown.Block.Code -> CodeBlock(block.text)

                is Markdown.Block.Quote -> Row {
                    Box(
                        Modifier
                            .width(3.dp)
                            .heightIn(min = 18.dp)
                            .background(MaterialTheme.colorScheme.outlineVariant)
                    )
                    Spacer(Modifier.width(8.dp))
                    Body(block.text, secondary = true)
                }

                Markdown.Block.Rule -> HorizontalDivider()

                is Markdown.Block.Table -> MarkdownTable(block)
            }
        }
    }
}

@Composable
private fun Body(text: String, secondary: Boolean) {
    Text(
        inline(text),
        style = MaterialTheme.typography.bodyMedium,
        color =
            if (secondary) MaterialTheme.colorScheme.onSurfaceVariant
            else MaterialTheme.colorScheme.onSurface,
    )
}

@Composable
private fun Marker(symbol: String, text: String, depth: Int, secondary: Boolean) {
    Row(Modifier.padding(start = (depth * 14).dp), verticalAlignment = Alignment.Top) {
        Text(
            symbol,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.width(20.dp),
        )
        Body(text, secondary)
    }
}

/**
 * A fenced block, scrolling sideways rather than wrapping.
 *
 * Wrapping a command makes it look like two commands, and a phone is narrow
 * enough that nearly every command would wrap. Horizontal scroll keeps the line
 * a line.
 */
@Composable
private fun CodeBlock(text: String) {
    Box(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(6.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHighest)
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = 10.dp, vertical = 8.dp)
    ) {
        Text(
            text,
            style = MaterialTheme.typography.bodySmall,
            fontFamily = FontFamily.Monospace,
        )
    }
}

/**
 * A pipe table.
 *
 * Column widths come from the content, so the whole thing scrolls sideways as
 * one rather than each row deciding for itself — which would misalign every
 * column and turn a table back into a wall of text.
 */
@Composable
private fun MarkdownTable(table: Markdown.Block.Table) {
    val widths = remember(table) {
        val columns = maxOf(table.header.size, table.rows.maxOfOrNull { it.size } ?: 0)
        IntArray(columns) { column ->
            val cells = listOf(table.header) + table.rows
            cells.maxOf { it.getOrNull(column)?.length ?: 0 }.coerceIn(3, 40)
        }
    }

    Box(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(6.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHighest)
            .horizontalScroll(rememberScrollState())
            .padding(8.dp)
    ) {
        Column {
            Row {
                for ((column, width) in widths.withIndex()) {
                    Text(
                        table.header.getOrNull(column).orEmpty(),
                        style = MaterialTheme.typography.labelMedium,
                        fontWeight = FontWeight.SemiBold,
                        modifier = Modifier.width((width * 8).dp).padding(end = 8.dp),
                    )
                }
            }
            HorizontalDivider(Modifier.padding(vertical = 4.dp))
            for (row in table.rows) {
                Row(Modifier.padding(vertical = 1.dp)) {
                    for ((column, width) in widths.withIndex()) {
                        Text(
                            inline(row.getOrNull(column).orEmpty()),
                            style = MaterialTheme.typography.bodySmall,
                            modifier = Modifier.width((width * 8).dp).padding(end = 8.dp),
                        )
                    }
                }
            }
        }
    }
}

/** Bold, italic, code spans and links, as one styled string. */
@Composable
fun inline(text: String): AnnotatedString {
    val codeBackground = MaterialTheme.colorScheme.surfaceContainerHighest
    val linkColor = MaterialTheme.colorScheme.primary
    return remember(text, codeBackground, linkColor) {
        buildAnnotatedString {
            for (span in Markdown.inline(text)) {
                val style = SpanStyle(
                    fontWeight = if (span.bold) FontWeight.Bold else null,
                    fontStyle = if (span.italic) FontStyle.Italic else null,
                    fontFamily = if (span.code) FontFamily.Monospace else null,
                    background = if (span.code) codeBackground else Color.Unspecified,
                    color = if (span.link != null) linkColor else Color.Unspecified,
                    textDecoration = if (span.link != null) TextDecoration.Underline else null,
                )
                withStyle(style) { append(span.text) }
            }
        }
    }
}
