package com.farcooler.ui

import android.content.ContentResolver
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
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
 * from a phone has to become a file on the machine the pane is on before the
 * agent can see it. The daemon writes it and types the path; this carries the
 * bytes there and says how far along it is.
 *
 * Always a transfer: a phone is never the machine the pane is on.
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
        mime: String,
        thumbnail: Bitmap?,
        terminal: String,
        core: ClientCore,
        scope: CoroutineScope,
    ) {
        val job = ImagePasteJob(thumbnail, data.size.toLong())
        if (data.size > LIMIT) {
            job.failure = "That image is too large to send. Images up to 16 MB work."
            jobs.add(job)
            return
        }

        val run = {
            job.failure = null
            job.sent = 0
            scope.launch {
                runCatching {
                    core.pasteImage(terminal, mime, data) { sent, total ->
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
            text.contains("image format") ->
                "Far Cooler can send PNG, JPEG, GIF, and WebP images."
            text.contains("image size") ->
                "That image is too large to send. Images up to 16 MB work."
            text.contains("not found") -> "That terminal isn't running anymore."
            else -> "Couldn't reach this machine."
        }
    }
}

/** What a picked image is, once it is something the agents can open. */
data class PickedImage(val data: ByteArray, val mime: String, val thumbnail: Bitmap?)

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
        return PickedImage(raw, mime, thumbnail)
    }

    // HEIC and anything else: both agents refuse it, so an untouched photo
    // would land as a file that exists, has a path, and cannot be read.
    val bitmap = thumbnail ?: return null
    val out = ByteArrayOutputStream()
    return if (bitmap.compress(Bitmap.CompressFormat.JPEG, 90, out)) {
        PickedImage(out.toByteArray(), "image/jpeg", bitmap)
    } else {
        null
    }
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
