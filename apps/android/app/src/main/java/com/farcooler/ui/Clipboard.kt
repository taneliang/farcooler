package com.farcooler.ui

import android.content.ClipData
import androidx.compose.ui.platform.Clipboard
import androidx.compose.ui.platform.ClipEntry

/**
 * The clipboard, as text.
 *
 * Compose's own clipboard is deliberately untyped — a clip can be an image, a
 * file, several things at once — and every use here is one string: a public key
 * going out, a command coming in. So the two conversions live here rather than
 * being repeated at each call site with a slightly different idea of what to do
 * with a clip that holds no text at all.
 */
suspend fun Clipboard.readText(): String? {
    val entry = getClipEntry() ?: return null
    val data = entry.clipData
    if (data.itemCount == 0) return null
    // `text` rather than `coerceToText`: coercing needs a Context to resolve a
    // content: URI and would go to the network for one, from a composable's
    // coroutine, to paste something that was never text. Everything this app
    // reads is a command or a key.
    return data.getItemAt(0).text?.toString()?.takeIf { it.isNotEmpty() }
}

suspend fun Clipboard.writeText(label: String, text: String) {
    setClipEntry(ClipEntry(ClipData.newPlainText(label, text)))
}
