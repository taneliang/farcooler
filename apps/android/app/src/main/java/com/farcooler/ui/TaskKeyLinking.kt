package com.farcooler.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.CustomAccessibilityAction
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
import com.farcooler.model.Plan
import com.farcooler.model.PlanReadState
import com.farcooler.model.TaskKeyCard
import com.farcooler.model.TaskKeyCards
import com.farcooler.model.TaskKeyIndex
import com.farcooler.model.TaskKeyLinks
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskKeyTarget
import com.farcooler.model.WorkspaceSummary
import com.farcooler.net.Connection

/**
 * What a screen's text links task keys to, and what opening one does (ov-196).
 * Provided where the runner is known, through [LocalTaskKeyLinker], and read by
 * [inline]. AgentKit's `TaskKeyLinker`.
 *
 * Equal when it links the same keys: [open] is left out, since a screen makes
 * a new one on every pass and a transcript's text shouldn't restyle for that.
 */
class TaskKeyLinker(
    val index: TaskKeyIndex,
    /** Each key's card (ov-299), from the same reads as [index]. */
    val cards: TaskKeyCards = TaskKeyCards.EMPTY,
    val open: (TaskKeyTarget) -> Unit,
) {
    /** The card for a key this linker links, on its own runner, or null. */
    fun card(key: String): TaskKeyCard? =
        if (index.targets.containsKey(key) && cards.runner == index.runner) cards.card(key) else null

    /** The card a task link names, when it's this linker's runner's. */
    fun cardFor(url: String): TaskKeyCard? {
        val (runner, key) = TaskKeyLinks.parse(url) ?: return null
        return if (runner == index.runner) card(key) else null
    }

    /** Open the task under [key], when this linker links it. */
    fun open(key: String) {
        index.targets[key]?.let(open)
    }

    /**
     * Open the task [url] names, when it's on this runner's boards: true when
     * it named one, opened or not, so a task link is never handed on.
     */
    fun follow(url: String): Boolean {
        if (TaskKeyLinks.parse(url) == null) return false
        TaskKeyLinks.target(url, index)?.let(open)
        return true
    }

    override fun equals(other: Any?): Boolean = other is TaskKeyLinker && other.index == index && other.cards == cards

    override fun hashCode(): Int = 31 * index.hashCode() + cards.hashCode()

    companion object {
        /** No keys, nothing to open: what text gets when no screen set one. */
        val NONE = TaskKeyLinker(TaskKeyIndex.EMPTY, TaskKeyCards.EMPTY) {}
    }
}

val LocalTaskKeyLinker = compositionLocalOf { TaskKeyLinker.NONE }

/** [connection]'s linker: its workspaces' prefixes and the boards it has read, each link navigating to its task. */
@Composable
fun rememberTaskKeyLinker(connection: Connection, navigate: (Route) -> Unit): TaskKeyLinker {
    val boards by connection.boards.collectAsStateWithLifecycle()
    val fleet by connection.fleet.collectAsStateWithLifecycle()
    val plans by connection.plans.states.collectAsStateWithLifecycle()
    val navigator by rememberUpdatedState(navigate)
    val runner = connection.host.id
    // Built once per read of the boards and plans, not per long press (ov-299).
    return remember(runner, fleet.workspaces, boards, plans) {
        val read = plans.mapNotNull { (id, state) -> (state as? PlanReadState.Loaded)?.let { id to it.plan } }.toMap()
        taskKeyLinker(runner, fleet.workspaces.orEmpty(), boards, read) { navigator(it) }
    }
}

/**
 * [runner]'s linker, each link handing [navigate] its task's own screen. Out of
 * [rememberTaskKeyLinker] so a unit test holds the wiring: which id goes where.
 */
fun taskKeyLinker(
    runner: String,
    workspaces: List<WorkspaceSummary>,
    boards: Map<String, TaskBoard>,
    plans: Map<String, Plan> = emptyMap(),
    navigate: (Route) -> Unit,
): TaskKeyLinker =
    TaskKeyLinker(TaskKeyIndex.of(runner, workspaces, boards), TaskKeyCards.of(runner, boards, plans)) { navigate(it.route()) }

/** Where a key's task opens: its own task screen, as a board row or History opens one. */
fun TaskKeyTarget.route(): Route = Route.BoardTask(runner, workspace, task)

/**
 * The tasks [text] links to, once each, in order: one accessibility action
 * apiece, "Open ov-190", for a row whose description replaces its text's own
 * links (ov-196).
 */
fun TaskKeyLinker.targets(text: AnnotatedString): List<TaskKeyTarget> =
    text.getLinkAnnotations(0, text.length)
        .mapNotNull { (it.item as? LinkAnnotation.Clickable)?.tag?.let { url -> TaskKeyLinks.target(url, index) } }
        .distinctBy { it.key }

/** [targets] as accessibility actions, each opening its task as a tap on the link would. */
fun TaskKeyLinker.actions(text: AnnotatedString): List<CustomAccessibilityAction> =
    targets(text).map { target ->
        // Its title too, when its card is known (ov-299).
        CustomAccessibilityAction("Open ${card(target.key)?.accessibilityLabel ?: target.key}") { open(target); true }
    }

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
        if ((span.code || span.link != null) && length > start) quoted.add(start until length)
    }
    if (linker.index.isEmpty) return@buildAnnotatedString
    val linkStyle = TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline))
    for (match in TaskKeyLinks.matches(spans.joinToString("") { it.text }, linker.index)) {
        if (quoted.any { it.first < match.end && match.start <= it.last }) continue
        val url = TaskKeyLinks.url(linker.index.runner, match.key)
        addLink(LinkAnnotation.Clickable(url, linkStyle) { linker.follow(url) }, match.start, match.end)
    }
}
