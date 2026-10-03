package com.farcooler.data

/**
 * Every theme the phone offers, assembled from every runner rather than
 * overwritten by whichever answered last.
 *
 * [Themes.merge] used to take ONE runner's list and rebuild the whole catalog
 * from it, `builtIn + thatRunner's themes`, called once per connection. With two
 * runners the catalog was whatever the runner that polled most recently
 * defines, so the other runner's theme left the list a moment after it arrived,
 * and [Themes.current] falls back when the stored name no longer resolves: the
 * whole app reverted to Nord, at random. The twin of AgentKit's
 * `ThemeCatalog.merged`, and both are pinned by
 * `test/fixtures/theme-catalog.json`.
 *
 * Two rules:
 *
 * - Every runner's themes are in the list at once.
 * - The answer does not depend on the order the runners answered in: they are
 *   folded in sorted runner order.
 */
object ThemeCatalog {
    /**
     * The built-ins, in their own order, with every runner's themes folded in.
     *
     * A runner's theme whose [name] matches one already in the list replaces it
     * in place rather than appending, so "Nord" stays where the eye found it.
     * Two runners defining one name has no right answer, so the answer is only
     * made stable: the last runner in sorted order wins.
     */
    fun <T> merged(builtIn: List<T>, byRunner: Map<String, List<T>>, name: (T) -> String): List<T> {
        val merged = builtIn.toMutableList()
        for (runner in byRunner.keys.sorted()) {
            for (theme in byRunner[runner].orEmpty()) {
                val index = merged.indexOfFirst { name(it) == name(theme) }
                if (index >= 0) merged[index] = theme else merged.add(theme)
            }
        }
        return merged
    }
}
