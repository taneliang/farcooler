package com.farcooler.model

import java.net.URLDecoder
import java.net.URLEncoder

/** Where a key's task is: enough for the app's own open-task path (ov-196). */
data class TaskKeyTarget(
    /** The app's own id for the runner, `Host.id`. The link carries it. */
    val runner: String,
    /** The workspace whose board has the task. */
    val workspace: String,
    /** The task's id. */
    val task: String,
    /** Its key, "ov-190". */
    val key: String,
)

/** One runner's keys that may become links: its workspaces' prefixes, and the tasks on the boards read so far. */
data class TaskKeyIndex(
    val runner: String,
    val prefixes: Set<String>,
    val targets: Map<String, TaskKeyTarget>,
) {
    val isEmpty: Boolean get() = prefixes.isEmpty() || targets.isEmpty()

    companion object {
        val EMPTY = TaskKeyIndex("", emptySet(), emptyMap())

        /** A runner's index from its workspaces and its boards, by workspace id. AgentKit's `TaskKeyIndex.init`. */
        fun of(runner: String, workspaces: List<WorkspaceSummary>, boards: Map<String, TaskBoard>): TaskKeyIndex {
            val targets = mutableMapOf<String, TaskKeyTarget>()
            for (workspace in boards.keys.sorted()) {
                for (row in boards[workspace]?.rows.orEmpty()) {
                    if (row.key !in targets) targets[row.key] = TaskKeyTarget(runner, workspace, row.id, row.key)
                }
            }
            return TaskKeyIndex(runner, workspaces.map { it.taskPrefix }.filter { it.isNotEmpty() }.toSet(), targets)
        }
    }
}

/**
 * Task keys in text as links (ov-196): "ov-190" in a task's intent or an
 * agent's reply opens that task. AgentKit's `TaskKeyLinks`, against the same
 * cases, `test/fixtures/task-key-links.json`.
 *
 * A word is a key only when it reads `<prefix>-<number>` with a boundary on
 * each side, its prefix is one of the runner's workspaces' task prefixes, and a
 * board the app has read has a task under it. So "utf-8", "x-86" and a key
 * nobody filed stay text.
 *
 * The link is `farcooler://task/<runner>/<key>`, and it never leaves the app:
 * it is followed by the screen that drew it, never handed to the system, which
 * would give `farcooler://` to whichever channel's app claimed it.
 */
object TaskKeyLinks {
    /** A key found in text: where it starts, in UTF-16 units (a Kotlin string's own), and what it says. */
    data class Match(val start: Int, val key: String) {
        val end: Int get() = start + key.length
    }

    /**
     * Every key in [text] under one of [prefixes] that [known] has, in order.
     *
     * A key is `<prefix>-<digits>`, the prefix an ASCII letter and then letters
     * or digits. Either side of it is the text's edge or anything but a letter,
     * a digit, `_` or `-`. The prefix's case is the workspace's own.
     */
    fun matches(text: String, prefixes: Set<String>, known: Set<String>): List<Match> {
        if (prefixes.isEmpty() || known.isEmpty()) return emptyList()
        val found = mutableListOf<Match>()
        var i = 0
        while (i < text.length) {
            if (!text[i].isAsciiLetter() || (i > 0 && joins(text[i - 1]))) {
                i += 1
                continue
            }
            var j = i + 1
            while (j < text.length && (text[j].isAsciiLetter() || text[j].isAsciiDigit())) j += 1
            var k = j + 1
            if (j >= text.length || text[j] != '-' || k >= text.length || !text[k].isAsciiDigit()) {
                i = j
                continue
            }
            while (k < text.length && text[k].isAsciiDigit()) k += 1
            if (k == text.length || !joins(text[k])) {
                val key = text.substring(i, k)
                if (text.substring(i, j) in prefixes && key in known) found.add(Match(i, key))
            }
            i = k
        }
        return found
    }

    /** [matches], against one runner's index. */
    fun matches(text: String, index: TaskKeyIndex): List<Match> = matches(text, index.prefixes, index.targets.keys)

    const val SCHEME = "farcooler"
    const val HOST = "task"

    /** `farcooler://task/<runner>/<key>`, each part escaped. */
    fun url(runner: String, key: String): String = "$SCHEME://$HOST/${escape(runner)}/${escape(key)}"

    /** The runner and key a task link names, or null for any other URL. */
    fun parse(url: String): Pair<String, String>? {
        val prefix = "$SCHEME://$HOST/"
        if (!url.startsWith(prefix, ignoreCase = true)) return null
        val parts = url.substring(prefix.length).split('/')
        if (parts.size != 2 || parts.any { it.isEmpty() }) return null
        return runCatching { unescape(parts[0]) to unescape(parts[1]) }.getOrNull()
    }

    /**
     * Whether a link may be opened: the web, mail and a task link, nothing
     * else. AgentKit's `Markdown.opens`: task text and replies are written by
     * agents, and an app's own scheme or `file:` must not open on a tap.
     */
    fun opens(url: String): Boolean {
        val scheme = url.substringBefore(':', missingDelimiterValue = "").lowercase()
        return scheme in setOf("http", "https", "mailto") || parse(url) != null
    }

    /** The task [url] names on [index]'s runner, or null: another runner's, a key it hasn't read, or no task link. */
    fun target(url: String, index: TaskKeyIndex): TaskKeyTarget? {
        val (runner, key) = parse(url) ?: return null
        return if (runner == index.runner) index.targets[key] else null
    }

    private fun escape(part: String): String = URLEncoder.encode(part, "UTF-8").replace("+", "%20")

    private fun unescape(part: String): String = URLDecoder.decode(part.replace("+", "%2B"), "UTF-8")

    private fun Char.isAsciiLetter(): Boolean = this in 'a'..'z' || this in 'A'..'Z'

    private fun Char.isAsciiDigit(): Boolean = this in '0'..'9'

    /** A letter or decimal digit in any script, `_` or `-`; a lone surrogate is neither. AgentKit's `joins`. */
    private fun joins(c: Char): Boolean = c == '-' || c == '_' || c.isLetterOrDigit()
}
