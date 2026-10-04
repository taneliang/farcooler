package com.farcooler.model

/**
 * Task keys in terminal output (ov-215): a key an agent prints opens its task,
 * where a URL printed there already opens. AgentKit's `TerminalTaskKeys`.
 *
 * The terminal core answers the whitespace-delimited word under a cell and
 * where the cell sits in it (`NativeVt.nativeWordAt`); which words are keys
 * stays [TaskKeyLinks.matches], against the same fixture.
 */
object TerminalLinks {
    /** The task link for the key [word] holds at [offset] (UTF-16 units), or null. */
    fun taskLink(word: String, offset: Int, index: TaskKeyIndex): String? {
        if (offset < 0 || index.isEmpty) return null
        val match = TaskKeyLinks.matches(word, index).firstOrNull { offset >= it.start && offset < it.end } ?: return null
        return TaskKeyLinks.url(index.runner, match.key)
    }

    /**
     * What a long press on a cell holds: the URL the core finds, else the task
     * link of a key, else null, which pastes. A URL wins, so a key inside one,
     * or inside a labelled hyperlink, stays the URL's.
     */
    fun resolve(urlAt: () -> String?, wordAt: () -> Pair<String, Int>?, index: TaskKeyIndex): String? {
        urlAt()?.let { return it }
        if (index.isEmpty) return null
        val (word, offset) = wordAt() ?: return null
        return taskLink(word, offset, index)
    }
}
