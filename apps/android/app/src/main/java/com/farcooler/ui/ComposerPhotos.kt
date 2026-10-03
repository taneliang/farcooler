package com.farcooler.ui

import com.farcooler.net.AgentStream

/**
 * The photos on the agent composer: the ones ready to send, and how many are
 * still being fitted to the prompt envelope.
 *
 * Fitting is off the main thread (see `PromptImageBudget`), so a photo is not
 * ready the instant it is picked. Without [preparing], a photo picked and then
 * sent inside that window missed the prompt it was picked for and silently
 * rode along with the next one. So the send waits: [canSend] is false while
 * any photo is still being prepared.
 */
data class ComposerPhotos(
    val ready: List<AgentStream.Attachment> = emptyList(),
    val preparing: Int = 0,
) {
    /** A photo was picked and is being fitted. */
    fun began(): ComposerPhotos = copy(preparing = preparing + 1)

    /** A fit finished: with the photo, or null when it couldn't be prepared. */
    fun finished(photo: AgentStream.Attachment?): ComposerPhotos =
        copy(ready = if (photo == null) ready else ready + photo, preparing = maxOf(0, preparing - 1))

    fun removed(index: Int): ComposerPhotos = copy(ready = ready.filterIndexed { i, _ -> i != index })

    /** Whether the send button may send [text] with these photos now. */
    fun canSend(text: String): Boolean = preparing == 0 && (text.isNotBlank() || ready.isNotEmpty())

    /** What is left once a prompt went: nothing ready, and nothing in flight, by [canSend]. */
    fun sent(): ComposerPhotos = ComposerPhotos()
}
