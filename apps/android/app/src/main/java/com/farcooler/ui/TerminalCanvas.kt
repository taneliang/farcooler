package com.farcooler.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import com.farcooler.core.TerminalGrid
import com.farcooler.core.TerminalPalette
import kotlin.math.floor
import kotlin.math.min

/** The cell grid a font produces, and the insets the canvas draws inside. */
data class CellMetrics(val width: Float, val height: Float) {
    companion object {
        val padding = 6.dp
    }
}

/**
 * Measure the cell a font produces, rather than assuming one.
 *
 * The device's font scale and display size both change what "13sp monospaced"
 * measures to, and a hard-coded guess would send the host a column count the
 * screen cannot actually show. Measured from a real glyph in the face that will
 * actually draw it: every cell is the same box in a monospaced face, so one
 * glyph's advance defines the grid.
 */
fun measureCell(measurer: TextMeasurer, style: TextStyle): CellMetrics {
    val layout: TextLayoutResult = measurer.measure("M", style)
    return CellMetrics(
        width = layout.size.width.toFloat(),
        height = layout.size.height.toFloat(),
    )
}

/**
 * The grid, drawn.
 *
 * Everything about what an escape sequence means happened before this ever sees
 * a byte — the session hands it a grid of already-resolved cells. What is left
 * is genuinely platform work: laying that grid out in pixels, and turning
 * touches into the bytes a program is waiting to read. It never parses an
 * escape sequence and never decides what an arrow key sends, matching the
 * contract the C header states for every renderer.
 */
@Composable
fun TerminalCanvas(
    grid: TerminalGrid,
    fontFamily: FontFamily,
    fontSize: Float,
    onSize: (columns: Int, rows: Int) -> Unit,
    onTap: () -> Unit,
    // Where the press landed, in cells, so the caller can ask what is there.
    onLongPress: (column: Int, row: Int) -> Unit,
    onScroll: (lines: Int, column: Int, row: Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current

    val regular = remember(fontFamily, fontSize) {
        TextStyle(fontFamily = fontFamily, fontSize = fontSize.sp, fontWeight = FontWeight.Normal)
    }
    val bold = remember(fontFamily, fontSize) {
        TextStyle(fontFamily = fontFamily, fontSize = fontSize.sp, fontWeight = FontWeight.Bold)
    }
    val cell = remember(regular) { measureCell(measurer, regular) }
    val paddingPx = with(density) { CellMetrics.padding.toPx() }

    var viewport by remember { mutableStateOf(IntSize.Zero) }

    // The size this screen can actually show, reported whenever either half of
    // the question changes: the viewport, or the cell a font produces. A
    // viewport that fits 80 columns at 13sp fits fewer at 18sp, and the pane
    // this screen asks the host to reshape should reflect the font actually on
    // screen rather than whatever it was measured at when it first appeared.
    LaunchedEffect(viewport, cell) {
        if (viewport.width <= 0 || cell.width <= 0f) return@LaunchedEffect
        val columns = floor((viewport.width - paddingPx * 2) / cell.width).toInt().coerceAtLeast(1)
        val rows = floor((viewport.height - paddingPx * 2) / cell.height).toInt().coerceAtLeast(1)
        onSize(columns, rows)
    }

    // The residual mismatch, scaled down rather than cropped.
    //
    // Almost never a straight reflow: the pane resizes itself to roughly fit,
    // but the two round trips that takes — asking the host, the host reflowing
    // tmux — leave a gap where the grid on screen is still the OLD size. Scale
    // is 1 the rest of the time.
    val contentWidth = paddingPx * 2 + grid.columns * cell.width
    val contentHeight = paddingPx * 2 + grid.rows * cell.height
    val scale =
        if (viewport.width <= 0 || contentWidth <= 0f || contentHeight <= 0f) 1f
        else min(min(viewport.width / contentWidth, viewport.height / contentHeight), 1f)

    val scaledCellWidth = cell.width * scale
    val scaledCellHeight = cell.height * scale
    val originX = paddingPx * scale
    val originY = paddingPx * scale

    val scaledRegular = remember(regular, scale) { regular.copy(fontSize = (fontSize * scale).sp) }
    val scaledBold = remember(bold, scale) { bold.copy(fontSize = (fontSize * scale).sp) }

    fun cellAt(offset: Offset): Pair<Int, Int> {
        if (scaledCellWidth <= 0f || scaledCellHeight <= 0f) return 0 to 0
        val column = floor((offset.x - originX) / scaledCellWidth).toInt()
        val row = floor((offset.y - originY) / scaledCellHeight).toInt()
        return column.coerceIn(0, (grid.columns - 1).coerceAtLeast(0)) to
            row.coerceIn(0, (grid.rows - 1).coerceAtLeast(0))
    }

    Box(
        modifier
            .fillMaxSize()
            .background(Color(TerminalPalette.BACKGROUND))
            // Reported from LAYOUT, not from the draw. Writing this inside the
            // Canvas lambda writes state that composition reads, from the phase
            // that runs after it — a redraw scheduling a recomposition
            // scheduling a redraw.
            .onSizeChanged { viewport = it }
            .pointerInput(Unit) {
                detectTapGestures(
                    onTap = { onTap() },
                    // The cell, not just the fact of the press: a long press
                    // over a link means something different from one over
                    // ordinary output, and only the caller can tell which.
                    onLongPress = { offset ->
                        val (column, row) = cellAt(offset)
                        onLongPress(column, row)
                    },
                )
            }
            .pointerInput(scaledCellHeight) {
                // Converted to whole lines, with the fractional remainder kept,
                // so a slow drag accumulates towards the next line instead of
                // being rounded away.
                var carry = 0f
                detectDragGestures(
                    onDragStart = { carry = 0f },
                    onDrag = { change, dragAmount ->
                        change.consume()
                        if (scaledCellHeight <= 0f) return@detectDragGestures
                        carry += dragAmount.y
                        val lines = (carry / scaledCellHeight).toInt()
                        if (lines == 0) return@detectDragGestures
                        carry -= lines * scaledCellHeight
                        val (column, row) = cellAt(change.position)
                        onScroll(lines, column, row)
                    },
                )
            }
    ) {
        Canvas(Modifier.fillMaxSize()) {
            drawGrid(
                grid = grid,
                measurer = measurer,
                regular = scaledRegular,
                bold = scaledBold,
                cellWidth = scaledCellWidth,
                cellHeight = scaledCellHeight,
                originX = originX,
                originY = originY,
            )
        }
    }
}

private fun DrawScope.drawGrid(
    grid: TerminalGrid,
    measurer: TextMeasurer,
    regular: TextStyle,
    bold: TextStyle,
    cellWidth: Float,
    cellHeight: Float,
    originX: Float,
    originY: Float,
) {
    drawRect(Color(TerminalPalette.BACKGROUND))
    if (cellWidth <= 0f || cellHeight <= 0f) return

    for (row in 0 until grid.rows) {
        val y = originY + row * cellHeight

        // Background first, batched into runs: a wide stretch of the terminal's
        // own default background is the common case, and skipping it entirely
        // is one comparison instead of one fill per cell across a mostly-empty
        // screen.
        var column = 0
        while (column < grid.columns) {
            val background = grid.background(row, column)
            var end = column + 1
            while (end < grid.columns && grid.background(row, end) == background) end += 1
            if (background != TerminalPalette.BACKGROUND) {
                drawRect(
                    color = Color(background),
                    topLeft = Offset(originX + column * cellWidth, y),
                    size = Size((end - column) * cellWidth, cellHeight),
                )
            }
            column = end
        }

        column = 0
        while (column < grid.columns) {
            val scalar = grid.character(row, column)
            val wide = grid.wide(row, column)
            // Blank cells are skipped rather than measured and painted four
            // thousand times a frame.
            if (scalar != 0 && scalar != ' '.code) {
                val text = String(Character.toChars(scalar))
                val style = if (grid.bold(row, column)) bold else regular
                drawText(
                    measurer.measure(text, style.copy(color = Color(grid.foreground(row, column)))),
                    topLeft = Offset(originX + column * cellWidth, y),
                )
            }
            // A wide character's neighbour is its own spacer; drawing it too
            // would put a second, blank glyph where the wide one already
            // reaches.
            column += if (wide) 2 else 1
        }
    }

    if (grid.cursorRow >= grid.rows || grid.cursorColumn >= grid.columns) return
    val cursorWide = grid.wide(grid.cursorRow, grid.cursorColumn)
    val cursorLeft = originX + grid.cursorColumn * cellWidth
    val cursorTop = originY + grid.cursorRow * cellHeight
    drawRect(
        color = Color(TerminalPalette.CURSOR),
        topLeft = Offset(cursorLeft, cursorTop),
        size = Size(if (cursorWide) cellWidth * 2 else cellWidth, cellHeight),
    )
    val under = grid.character(grid.cursorRow, grid.cursorColumn)
    if (under != 0 && under != ' '.code) {
        drawText(
            measurer.measure(
                String(Character.toChars(under)),
                regular.copy(color = Color(TerminalPalette.BACKGROUND)),
            ),
            topLeft = Offset(cursorLeft, cursorTop),
        )
    }
}

/**
 * The keyboard, attached to something invisible.
 *
 * A one-pixel view rather than one filling the terminal, unlike the iOS
 * equivalent: Android hands the IME to whatever has focus and shows it when
 * asked, with no size requirement at all, so the gestures can stay in Compose
 * where they belong instead of being routed around a view that owns every
 * touch. Kept at 1 dp rather than 0 so it is laid out and can hold focus.
 */
@Composable
fun TerminalKeyboardAnchor(
    focusRequest: Int,
    dismissRequest: Int,
    onText: (String) -> Unit,
    onKey: (Int, Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    var view by remember { mutableStateOf<TerminalInputView?>(null) }

    AndroidView(
        factory = { context ->
            TerminalInputView(context).also {
                it.onText = onText
                it.onKey = onKey
                view = it
            }
        },
        update = {
            it.onText = onText
            it.onKey = onKey
        },
        modifier = modifier.size(1.dp),
    )

    LaunchedEffect(focusRequest) {
        if (focusRequest > 0) view?.showKeyboard()
    }
    LaunchedEffect(dismissRequest) {
        if (dismissRequest > 0) view?.hideKeyboard()
    }
}
