package com.farcooler.net

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.util.Base64
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull

/**
 * Where a prompt's images come from (ov-454, ov-458): the runner, which reads each
 * back from the agent's transcript (`agent.image`), so an image pasted in the
 * terminal shows as one sent from a composer does. The Mac's `PromptImageSource`.
 */
fun interface PromptImageSource {
    /** Image [index] of the prompt on turn row [row], its bytes whole. */
    suspend fun image(terminal: String, row: String, index: Int): ByteArray
}

/** The runner's `agent.image`, over this phone's client core. */
class CorePromptImages(private val core: ClientCall) : PromptImageSource {
    override suspend fun image(terminal: String, row: String, index: Int): ByteArray {
        val answer = core.call(
            "agent.image",
            buildJsonObject {
                put("terminal", JsonPrimitive(terminal))
                put("row", JsonPrimitive(row))
                put("index", JsonPrimitive(index))
            },
        )
        val text = (answer["base64"] as? JsonPrimitive)?.contentOrNull
            ?: throw IllegalStateException("The image couldn’t be read.")
        return Base64.getDecoder().decode(text)
    }
}

/**
 * One pane's prompt images: each fetched once, kept for its thumbnail and for the
 * full-size view. The Mac's `PromptImageStore`.
 */
class PromptImageStore(private val terminal: String, private val files: PaneImageFiles? = null) {
    /** What a thumbnail shows. */
    sealed interface State {
        data object Loading : State
        class Shown(val bytes: ByteArray) : State
        data object Failed : State
    }

    /** The runner, where it serves `agent.image`; null, and no thumbnails are drawn, where it doesn't. */
    var source by mutableStateOf<PromptImageSource?>(null)

    private val states = mutableStateMapOf<String, State>()
    private val asked = HashSet<String>()

    /**
     * The names of this pane's files on disk, read once off the main thread as the store is made, so what a redraw asks
     * ([isKept]) is answered from memory.
     */
    private var keptFiles by mutableStateOf<Set<String>>(emptySet())
    private val indexing: Thread? = files?.let { disk ->
        Thread { keptFiles = disk.names(terminal).toSet() }.apply { isDaemon = true; start() }
    }

    /** Wait for the names on disk to be read. For tests. */
    fun awaitIndex() {
        indexing?.join()
    }

    fun state(row: String, index: Int): State = states[key(row, index)] ?: State.Loading

    /** Fetch image [index] of [row] once; again only after it failed and the row is shown anew. */
    suspend fun load(row: String, index: Int) {
        val key = key(row, index)
        // From disk where it is kept (ov-622): after a restart, and with no runner to ask.
        val held = files?.takeIf { cacheable(row) && isKept(row, index) }
            ?.let { disk -> kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { disk.read(fileName(key), terminal) } }
        if (held != null) {
            synchronized(asked) { asked.add(key) }
            states[key] = State.Shown(held)
            return
        }
        val source = source ?: return
        synchronized(asked) { if (!asked.add(key)) return }
        states[key] = State.Loading
        try {
            val bytes = source.image(terminal, row, index)
            files?.takeIf { cacheable(row) }?.let { disk ->
                val kept = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { disk.write(bytes, fileName(key), terminal) }
                if (kept) keptFiles = keptFiles + fileName(key)
            }
            states[key] = State.Shown(bytes)
        } catch (e: kotlinx.coroutines.CancellationException) {
            synchronized(asked) { asked.remove(key) }
            throw e
        } catch (e: Exception) {
            states[key] = State.Failed
            synchronized(asked) { asked.remove(key) }
        }
    }

    /** Whether image [index] of [row] is on disk, so it can be drawn with no runner to ask (ov-622). */
    fun isKept(row: String, index: Int): Boolean = cacheable(row) && fileName(key(row, index)) in keptFiles

    /** The pane is gone: its cached images go with it. */
    fun forget() {
        files?.let { disk -> Thread { disk.removePane(terminal) }.apply { isDaemon = true }.start() }
    }

    private fun fileName(key: String): String {
        var hash = 2166136261L
        for (byte in key.toByteArray(Charsets.UTF_8)) hash = ((hash xor (byte.toLong() and 0xFF)) * 16777619L) and 0xFFFFFFFFL
        return key.map { if (it.isLetterOrDigit() || it == '-') it else '-' }.joinToString("") + "-" + java.lang.Long.toHexString(hash) + ".img"
    }

    companion object {
        fun key(row: String, index: Int) = "$row#$index"

        /**
         * Whether the images of turn [row] are kept at all. A resumed session's turns are numbered from 0 again
         * (`turn:resumed:{seq}`), so the same id names another turn in another session and a cached image would show
         * under the wrong message (ov-622).
         */
        fun cacheable(row: String) = !row.startsWith("turn:resumed:")
    }
}
