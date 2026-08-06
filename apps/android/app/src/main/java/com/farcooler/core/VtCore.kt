package com.farcooler.core

/**
 * Kotlin's view of the Rust terminal core.
 *
 * A thin, safe wrapper: it owns the handle's lifetime, converts between Kotlin
 * and the flat arrays the JNI layer hands back, and nothing else. Every
 * decision about what bytes mean — what colour an escape sequence produces,
 * what an arrow key encodes to under the program's current mode — lives on the
 * other side of this boundary. That is what lets this renderer and the Mac's
 * answer to the same core without agreeing on a single line of emulator logic.
 *
 * Not thread-safe by design, matching the contract the C header documents for
 * every renderer. [com.farcooler.net.TerminalSession] confines it to one
 * dispatcher.
 */
class VtCore(columns: Int, rows: Int) {
    private var handle: Long =
        if (NativeLibrary.loaded) NativeVt.nativeNew(columns, rows) else 0

    fun free() {
        if (handle == 0L) return
        NativeVt.nativeFree(handle)
        handle = 0
    }

    fun feed(bytes: ByteArray) {
        if (handle == 0L || bytes.isEmpty()) return
        NativeVt.nativeFeed(handle, bytes)
    }

    fun resize(columns: Int, rows: Int) {
        if (handle == 0L) return
        NativeVt.nativeResize(handle, columns, rows)
    }

    /** Scroll the view. Positive goes back into history. */
    fun scroll(lines: Int) {
        if (handle == 0L) return
        NativeVt.nativeScroll(handle, lines)
    }

    /**
     * Jump back to the live screen. Call this on input: typing into a
     * scrolled-back view would show the user nothing of what they typed.
     */
    /**
     * Recolour every cell the next snapshot produces.
     *
     * Colours resolve when a snapshot is taken rather than when bytes arrive,
     * so this recolours scrollback too — switching themes is instant, not a
     * repaint of history the terminal no longer holds.
     */
    fun setPalette(colors: IntArray): Boolean {
        val h = handle
        if (h == 0L) return false
        return NativeVt.nativeSetPalette(h, colors)
    }

    fun scrollToBottom() {
        if (handle == 0L) return
        NativeVt.nativeScrollToBottom(handle)
    }

    /** A copy of the screen, safe to hold and hand to Compose. */
    fun snapshot(): TerminalGrid? {
        if (handle == 0L) return null
        val flat = NativeVt.nativeSnapshot(handle) ?: return null
        return TerminalGrid.from(flat)
    }

    /**
     * Bytes the program wants written back to the pty.
     *
     * Cursor-position reports and mouse replies must reach the program or a
     * full-screen agent sits waiting for an answer that never comes.
     */
    fun takeWrites(): ByteArray {
        if (handle == 0L) return ByteArray(0)
        return NativeVt.nativeTakeWrites(handle) ?: ByteArray(0)
    }

    fun title(): String? = if (handle == 0L) null else NativeVt.nativeTitle(handle)

    /**
     * Text the program asked to put on the clipboard (OSC 52), or null.
     *
     * This matters more here than on the Mac: there is no text selection in
     * this renderer, so OSC 52 is the only way anything on screen reaches the
     * clipboard at all.
     *
     * There is no counterpart that reads the clipboard. A program asking for
     * its contents is refused inside the core — copy is a program handing you
     * something, paste is a program taking something, and Far Cooler runs
     * agents on machines nobody is watching.
     */
    fun takeClipboard(): String? =
        if (handle == 0L) null else NativeVt.nativeTakeClipboard(handle)

    /**
     * The URL under a cell of the screen as currently shown, or null.
     *
     * The core decides what counts as a URL and which schemes may be opened;
     * terminal output is not trusted input, and keeping the allowlist there
     * makes it one list rather than three.
     */
    fun urlAt(row: Int, column: Int): String? =
        if (handle == 0L || row < 0 || column < 0) null
        else NativeVt.nativeUrlAt(handle, row, column)

    /**
     * Encode a keystroke for the program currently running.
     *
     * The core answers because the answer depends on modes it holds: an arrow
     * key is different bytes under application cursor mode, and a Ctrl modifier
     * turns a printable scalar into a control code (Ctrl-C is `c` with
     * [Vt.MOD_CTRL], not a separate key). Neither is this file's to decide.
     */
    fun encodeKey(key: Int, modifiers: Int): ByteArray {
        if (handle == 0L) return ByteArray(0)
        return NativeVt.nativeEncodeKey(handle, key, modifiers) ?: ByteArray(0)
    }

    /** Null when the program does not want the event — handle it locally. */
    fun encodeMouse(button: Int, action: Int, column: Int, row: Int, modifiers: Int): ByteArray? {
        if (handle == 0L) return null
        return NativeVt.nativeEncodeMouse(handle, button, action, column, row, modifiers)
    }

    /**
     * Encode pasted text, bracketing it if the program asked for that —
     * without which an editor auto-indents every pasted line and a shell runs
     * each newline as a command.
     */
    fun encodePaste(text: String): ByteArray {
        if (handle == 0L) return ByteArray(0)
        return NativeVt.nativeEncodePaste(handle, text) ?: ByteArray(0)
    }
}

/**
 * A plain-data copy of one screen.
 *
 * The JNI layer hands back one flat `int[]` rather than an array of objects,
 * and it is unpacked here into four parallel arrays rather than a list of cell
 * objects, for the same reason: an 80×24 screen is 1,920 cells redrawn several
 * times a second, and per-cell objects are exactly the shape of garbage that
 * makes a scrolling terminal stutter.
 */
class TerminalGrid(
    val columns: Int,
    val rows: Int,
    val cursorRow: Int,
    val cursorColumn: Int,
    val cursorVisible: Boolean,
    val displayOffset: Int,
    val historySize: Int,
    private val characters: IntArray,
    private val foregrounds: IntArray,
    private val backgrounds: IntArray,
    private val flags: IntArray,
) {
    private fun index(row: Int, column: Int) = row * columns + column

    /** The Unicode scalar in a cell, or 0 where nothing was written. */
    fun character(row: Int, column: Int): Int = characters[index(row, column)]

    /**
     * Already resolved for [Vt.FLAG_INVERSE] — the core reports foreground and
     * background separately from the flag, and a renderer that forgot to swap
     * them would draw reverse-video text invisibly on itself.
     */
    fun foreground(row: Int, column: Int): Int {
        val i = index(row, column)
        val packed = if (inverse(i)) backgrounds[i] else foregrounds[i]
        return opaque(packed)
    }

    fun background(row: Int, column: Int): Int {
        val i = index(row, column)
        val packed = if (inverse(i)) foregrounds[i] else backgrounds[i]
        return opaque(packed)
    }

    fun bold(row: Int, column: Int): Boolean = flags[index(row, column)] and Vt.FLAG_BOLD != 0

    /**
     * A double-width character. The next column is its spacer; a renderer that
     * drew both would show the glyph twice.
     */
    fun wide(row: Int, column: Int): Boolean = flags[index(row, column)] and Vt.FLAG_WIDE != 0

    /**
     * The same screen with the caret somewhere else.
     *
     * For a screen built from a capture rather than a live stream: a capture is
     * text, so feeding it leaves the emulator's caret wherever the last
     * character landed — the bottom left — rather than at the prompt someone is
     * typing into. The host reports where it really is, and this is where that
     * answer is applied. The cell arrays are shared rather than copied; nothing
     * mutates them after construction.
     */
    fun withCursor(row: Int, column: Int) = TerminalGrid(
        columns = columns,
        rows = rows,
        cursorRow = row.coerceIn(0, (rows - 1).coerceAtLeast(0)),
        cursorColumn = column.coerceIn(0, (columns - 1).coerceAtLeast(0)),
        cursorVisible = cursorVisible,
        displayOffset = displayOffset,
        historySize = historySize,
        characters = characters,
        foregrounds = foregrounds,
        backgrounds = backgrounds,
        flags = flags,
    )

    private fun inverse(i: Int) = flags[i] and Vt.FLAG_INVERSE != 0

    private fun opaque(packed: Int) = packed or 0xFF000000.toInt()

    companion object {
        private const val HEADER = 7

        fun from(flat: IntArray): TerminalGrid? {
            if (flat.size < HEADER) return null
            val columns = flat[0]
            val rows = flat[1]
            val count = columns * rows
            if (columns <= 0 || rows <= 0 || flat.size < HEADER + count * 4) return null

            val characters = IntArray(count)
            val foregrounds = IntArray(count)
            val backgrounds = IntArray(count)
            val flags = IntArray(count)
            var read = HEADER
            for (i in 0 until count) {
                characters[i] = flat[read++]
                foregrounds[i] = flat[read++]
                backgrounds[i] = flat[read++]
                flags[i] = flat[read++]
            }

            return TerminalGrid(
                columns = columns,
                rows = rows,
                // Clamped rather than trusted: the cursor position can come
                // from a separate call than the screen dump (see
                // `TerminalSession.render`), and the two can race a resize by
                // one poll interval.
                cursorRow = flat[2].coerceIn(0, (rows - 1).coerceAtLeast(0)),
                cursorColumn = flat[3].coerceIn(0, (columns - 1).coerceAtLeast(0)),
                cursorVisible = flat[4] != 0,
                displayOffset = flat[5],
                historySize = flat[6],
                characters = characters,
                foregrounds = foregrounds,
                backgrounds = backgrounds,
                flags = flags,
            )
        }
    }
}

/**
 * Colours the core did not resolve, because they belong to this screen rather
 * than to any program running on the host: the fill behind a short last row and
 * the cursor block. Values mirror the Mac and iOS apps' so the same terminal
 * looks like the same terminal on all three.
 */
/**
 * The colours the renderer owns, read from the theme in force.
 *
 * These were constants, which is what made the palette unchangeable without
 * reinstalling. Cell colours still come from the core already resolved; this is
 * the chrome around them.
 */
object TerminalPalette {
    val BACKGROUND: Int get() =
        com.farcooler.data.Themes.opaque(com.farcooler.data.Themes.current.background)
    val CURSOR: Int get() =
        com.farcooler.data.Themes.opaque(com.farcooler.data.Themes.current.cursor)
}
