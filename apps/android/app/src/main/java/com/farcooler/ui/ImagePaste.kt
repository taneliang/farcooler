package com.farcooler.ui

import android.content.ContentResolver
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.provider.OpenableColumns
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import com.farcooler.core.ClientCore
import java.io.ByteArrayOutputStream
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * Pasting an image into a terminal.
 *
 * A terminal takes bytes and an agent running in one opens a path, so an image
 * from a phone has to become a file on the runner the pane is on before the
 * agent can see it. The daemon writes it and types the path; this carries the
 * bytes there and says how far along it is.
 *
 * Always a transfer: a phone is never the runner the pane is on.
 */

/** The largest image the daemon accepts, checked here so a slow link is not spent learning it. */
private const val LIMIT: Long = 16L * 1024 * 1024

/** One image on its way into a pane. */
class ImagePasteJob(val thumbnail: Bitmap?, total: Long) {
    var sent by mutableStateOf(0L)
    var total by mutableStateOf(total)
    var failure by mutableStateOf<String?>(null)
    var retry: (() -> Unit)? = null

    val fraction: Float
        get() = if (total <= 0) 0f else (sent.toFloat() / total.toFloat()).coerceIn(0f, 1f)
}

/** The jobs for one screen. */
class ImagePasteQueue {
    val jobs = mutableStateListOf<ImagePasteJob>()

    fun send(
        data: ByteArray,
        name: String,
        mime: String,
        thumbnail: Bitmap?,
        terminal: String,
        core: ClientCore,
        scope: CoroutineScope,
    ) {
        val job = ImagePasteJob(thumbnail, data.size.toLong())
        if (data.size > LIMIT) {
            job.failure = "That file is too large to send. Files up to 16 MB work."
            jobs.add(job)
            return
        }

        val run = {
            job.failure = null
            job.sent = 0
            scope.launch {
                runCatching {
                    core.pasteFile(terminal, name, mime, data) { sent, total ->
                        job.sent = sent
                        if (total > 0) job.total = total
                    }
                }
                    .onSuccess { jobs.remove(job) }
                    .onFailure { job.failure = messageFor(it) }
            }
            Unit
        }
        job.retry = run
        jobs.add(job)
        run()
    }

    /** Show a failure that happened before there was anything to transfer. */
    fun reject(message: String) {
        jobs.add(ImagePasteJob(null, 0).also { it.failure = message })
    }

    fun dismiss(job: ImagePasteJob) {
        jobs.remove(job)
    }

    /**
     * Turn the core's answer into something worth showing someone.
     *
     * The core's own text is developer-facing, and none of it belongs on a
     * phone screen sitting over a terminal.
     */
    private fun messageFor(error: Throwable): String {
        val text = (error.message ?: "").lowercase()
        return when {
            // Two causes, indistinguishable on the wire: a daemon too old to
            // know the method, or a pane that closed before the path could be
            // typed. Named together rather than guessed at.
            text.contains("predates this") ->
                "That terminal may have closed, or this runner’s Far Cooler may be too old."
            text.contains("file size") ->
                "That file is too large to send. Files up to 16 MB work."
            text.contains("not found") -> "That terminal isn’t running anymore."
            else -> "Couldn’t reach this runner."
        }
    }
}

/** What a picked image is, with the name it had where it came from. */
data class PickedImage(
    val data: ByteArray,
    val name: String,
    val mime: String,
    val thumbnail: Bitmap?,
)

/**
 * Read a picked image, re-encoding only when it is a format an agent refuses.
 *
 * Bytes are preferred over a re-encode: a screenshot's whole point is usually
 * small text, and a round trip through a bitmap costs exactly the detail that
 * makes it worth sending.
 */
fun readPickedImage(resolver: ContentResolver, uri: Uri): PickedImage? {
    val raw = runCatching { resolver.openInputStream(uri)?.use { it.readBytes() } }.getOrNull()
        ?: return null
    val thumbnail = runCatching { BitmapFactory.decodeByteArray(raw, 0, raw.size) }.getOrNull()

    val mime = resolver.getType(uri) ?: ""
    if (mime in setOf("image/png", "image/jpeg", "image/gif", "image/webp")) {
        return PickedImage(raw, displayName(resolver, uri, mime), mime, thumbnail)
    }

    // HEIC and anything else: both agents refuse it, so an untouched photo
    // would land as a file that exists, has a path, and cannot be read.
    val bitmap = thumbnail ?: return null
    val out = ByteArrayOutputStream()
    return if (bitmap.compress(Bitmap.CompressFormat.JPEG, 90, out)) {
        PickedImage(out.toByteArray(), displayName(resolver, uri, "image/jpeg"), "image/jpeg", bitmap)
    } else {
        null
    }
}

/**
 * What the picker's content URI is called, if anything.
 *
 * A `content://` URI carries no path worth reading, so the display name is
 * asked for separately. Worth the query: `receipt-2026.pdf` in a prompt says
 * what the agent is looking at, and the fallback says nothing.
 */
private fun displayName(resolver: ContentResolver, uri: Uri, mime: String): String {
    val name = runCatching {
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        }
    }.getOrNull()?.takeIf { it.isNotBlank() }
    if (name != null) return name
    return if (mime == "image/jpeg") "photo.jpg" else "photo.png"
}

/** The chips for one screen, stacked over the bottom of the terminal. */
@Composable
fun ImagePasteChips(queue: ImagePasteQueue, modifier: Modifier = Modifier) {
    Column(
        modifier.padding(bottom = 12.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        queue.jobs.forEach { job -> ImagePasteChip(job) { queue.dismiss(job) } }
    }
}

@Composable
private fun ImagePasteChip(job: ImagePasteJob, onDismiss: () -> Unit) {
    Surface(
        shape = MaterialTheme.shapes.medium,
        color = MaterialTheme.colorScheme.surfaceVariant,
        tonalElevation = 3.dp,
        shadowElevation = 3.dp,
    ) {
        Row(
            Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            job.thumbnail?.let {
                androidx.compose.foundation.Image(
                    bitmap = it.asImageBitmap(),
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.size(26.dp),
                )
            }
            val failure = job.failure
            if (failure != null) {
                Text(failure, style = MaterialTheme.typography.bodySmall)
                TextButton(onClick = { job.retry?.invoke() }) { Text("Retry") }
                TextButton(onClick = onDismiss) { Text("Cancel") }
            } else {
                Text("Sending…", style = MaterialTheme.typography.bodySmall)
                CircularProgressIndicator(
                    progress = { job.fraction },
                    modifier = Modifier.size(18.dp),
                )
            }
        }
    }
}
