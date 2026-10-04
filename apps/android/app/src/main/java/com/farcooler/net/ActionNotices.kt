package com.farcooler.net

import com.farcooler.core.DisconnectedException
import com.farcooler.core.refusalWord
import com.farcooler.model.troubleAfter
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow

/**
 * The sentences for host calls that failed, to be shown once each (ov-180).
 *
 * A stream of events rather than state: a failed hide is something that
 * happened, not a condition the screen is in, so a screen that was not
 * collecting when it happened has nothing to catch up on, and one that is
 * collecting shows it once. The newest four are kept if the collector is slow.
 * Never a raw runner error: [run] says the step that failed, and adds the
 * runner's reason only where `RunnerRefusal` has a sentence for its word.
 */
class ActionNotices {
    private val _sentences = MutableSharedFlow<String>(
        extraBufferCapacity = 4,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )

    val sentences: SharedFlow<String> = _sentences.asSharedFlow()

    /**
     * Make [call]; if it fails, say [context] and return false. [because] holds
     * this call's own plain reasons by the runner's word, for refusals the
     * shared table has no sentence for.
     */
    suspend fun run(
        context: String,
        because: Map<String, String> = emptyMap(),
        call: suspend () -> Unit,
    ): Boolean {
        try {
            call()
            return true
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            _sentences.tryEmit(
                if (e is DisconnectedException) "$context The connection to this runner dropped."
                else if (e.refusalWord in because) "$context ${because.getValue(e.refusalWord!!)}"
                else troubleAfter(e.refusalWord, e.message, context).sentence
            )
            return false
        }
    }
}
