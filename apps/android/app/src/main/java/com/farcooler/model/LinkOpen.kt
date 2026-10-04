package com.farcooler.model

/**
 * Opening a link in whatever app takes it, and saying so when none does.
 *
 * `UriHandler.openUri` throws when no app on the phone handles the scheme, and
 * "Open" that did nothing at all was the whole of what a person saw (ov-180).
 */
object LinkOpen {
    const val NO_APP = "No app on this phone can open this link."

    /** Null when [open] took the link, else the sentence to show. */
    fun open(link: String, open: (String) -> Unit): String? =
        try {
            open(link)
            null
        } catch (e: Exception) {
            NO_APP
        }
}
