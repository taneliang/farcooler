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
class PromptImageStore(private val terminal: String) {
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

    fun state(row: String, index: Int): State = states[key(row, index)] ?: State.Loading

    /** Fetch image [index] of [row] once; again only after it failed and the row is shown anew. */
    suspend fun load(row: String, index: Int) {
        val key = key(row, index)
        val source = source ?: return
        synchronized(asked) { if (!asked.add(key)) return }
        states[key] = State.Loading
        try {
            states[key] = State.Shown(source.image(terminal, row, index))
        } catch (e: kotlinx.coroutines.CancellationException) {
            synchronized(asked) { asked.remove(key) }
            throw e
        } catch (e: Exception) {
            states[key] = State.Failed
            synchronized(asked) { asked.remove(key) }
        }
    }

    companion object {
        fun key(row: String, index: Int) = "$row#$index"
    }
}
