package com.farcooler.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import com.farcooler.model.Markdown
import com.farcooler.model.TaskKeyIndex
import com.farcooler.model.TaskKeyLinks
import com.farcooler.model.TaskKeyTarget
import com.farcooler.net.Connection

/**
 * What a screen's text links task keys to, and what opening one does (ov-196).
 * Provided where the runner is known, through [LocalTaskKeyLinker], and read by
 * [inline]. AgentKit's `TaskKeyLinker`.
 *
 * Equal when it links the same keys: [open] is left out, since a screen makes
 * a new one on every pass and a transcript's text shouldn't restyle for that.
 */
class TaskKeyLinker(val index: TaskKeyIndex, val open: (TaskKeyTarget) -> Unit) {
    /**
     * Open the task [url] names, when it's on this runner's boards: true when
     * it named one, opened or not, so a task link is never handed on.
     */
    fun follow(url: String): Boolean {
        if (TaskKeyLinks.parse(url) == null) return false
        TaskKeyLinks.target(url, index)?.let(open)
        return true
    }

    override fun equals(other: Any?): Boolean = other is TaskKeyLinker && other.index == index

    override fun hashCode(): Int = index.hashCode()

    companion object {
        /** No keys, nothing to open: what text gets when no screen set one. */
        val NONE = TaskKeyLinker(TaskKeyIndex.EMPTY) {}
    }
}

val LocalTaskKeyLinker = compositionLocalOf { TaskKeyLinker.NONE }

/** [connection]'s linker: its workspaces' prefixes and the boards it has read, opening through [open]. */
@Composable
fun rememberTaskKeyLinker(connection: Connection, open: (TaskKeyTarget) -> Unit): TaskKeyLinker {
    val boards by connection.boards.collectAsStateWithLifecycle()
    val fleet by connection.fleet.collectAsStateWithLifecycle()
    val opener by rememberUpdatedState(open)
    val runner = connection.host.id
    return remember(runner, fleet.workspaces, boards) {
        TaskKeyLinker(TaskKeyIndex.of(runner, fleet.workspaces.orEmpty(), boards)) { opener(it) }
    }
}

/** Where a key's task opens: its own task screen, as a board row or History opens one. */
fun TaskKeyTarget.route(): Route = Route.BoardTask(runner, workspace, task)

/**
 * A line's inline spans as one styled string, with each task key [linker]
 * knows made a link to its task. A key in code or in a link's words is left
 * alone: quoted, or the link's own.
 */
fun inlineAnnotated(
    spans: List<Markdown.Span>,
    linker: TaskKeyLinker,
    codeBackground: Color,
    linkColor: Color,
): AnnotatedString = buildAnnotatedString {
    val quoted = mutableListOf<IntRange>()
    for (span in spans) {
        val style = SpanStyle(
            fontWeight = if (span.bold) FontWeight.Bold else null,
            fontStyle = if (span.italic) FontStyle.Italic else null,
            fontFamily = if (span.code) FontFamily.Monospace else null,
            background = if (span.code) codeBackground else Color.Unspecified,
            color = if (span.link != null) linkColor else Color.Unspecified,
            textDecoration = if (span.link != null) TextDecoration.Underline else null,
        )
        val start = length
        withStyle(style) { append(span.text) }
        if (span.code || span.link != null) quoted.add(start until length)
    }
    if (linker.index.isEmpty) return@buildAnnotatedString
    val linkStyle = TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline))
    for (match in TaskKeyLinks.matches(spans.joinToString("") { it.text }, linker.index)) {
        if (quoted.any { it.first < match.end && match.start <= it.last }) continue
        val url = TaskKeyLinks.url(linker.index.runner, match.key)
        addLink(LinkAnnotation.Clickable(url, linkStyle) { linker.follow(url) }, match.start, match.end)
    }
}
