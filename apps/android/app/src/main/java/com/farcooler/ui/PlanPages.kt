package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.outlined.Article
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.BoardPage
import com.farcooler.model.Fleet
import com.farcooler.model.PageDestination
import com.farcooler.model.PageShelf
import com.farcooler.model.PageSpeech
import com.farcooler.model.PageWords
import com.farcooler.model.PageWorld
import com.farcooler.model.Plan
import com.farcooler.model.PlanPage
import com.farcooler.model.PlanWords
import com.farcooler.model.TaskRow
import com.farcooler.net.PageListState
import com.farcooler.net.TerminalRef

// Orchestrator pages in the Plan view on Android (ov-269 design 6.1 and 6.5,
// ov-285): the Pages section after Themes, a page pushed, and the sections a
// theme's page draws for the pages anchored to it. The blocks are
// `PageView.kt`'s; this file is where they live and what a reference opens.

/** What the Plan view needs to list a board's pages. Null on a runner without `board_pages`, which shows no section. */
data class PagesHook(val state: PageListState?, val now: Long, val onRetry: () -> Unit)

/** What the Pages section shows: nothing, a read that failed, or the pages listed outside their themes. */
sealed interface PagesSection {
    /** The count its header draws; null when it isn't known, never a zero that says there's nothing. */
    val count: Int?

    data object None : PagesSection { override val count: Int? get() = null }
    data object Unavailable : PagesSection { override val count: Int? get() = null }
    data class Listed(val pages: List<BoardPage>) : PagesSection { override val count: Int get() = pages.size }

    companion object {
        fun of(state: PageListState?, plan: Plan): PagesSection = when (state) {
            null -> None
            PageListState.Unavailable -> Unavailable
            is PageListState.Loaded -> PageShelf.listed(state.pages, plan).let { if (it.isEmpty()) None else Listed(it) }
        }
    }
}

/** The Pages section: one row per page that isn't drawn inside a theme, each opening the page, pushed. */
fun LazyListScope.pageItems(hook: PagesHook, plan: Plan, onOpen: (PlanPage) -> Unit) {
    when (val section = PagesSection.of(hook.state, plan)) {
        PagesSection.None -> Unit
        PagesSection.Unavailable -> {
            item(key = "plan/pages") { PlanHeader("Pages", section.count, Modifier.testTag("plan-pages")) }
            item(key = "plan/pages/unavailable") {
                Column(Modifier.testTag("plan-pages-unavailable")) {
                    PlanNotice(PageWords.COULDNT_READ, null)
                    TextButton(
                        onClick = hook.onRetry,
                        contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp),
                        modifier = Modifier.testTag("plan-pages-retry"),
                    ) { Text(PlanWords.TRY_AGAIN) }
                }
            }
        }
        is PagesSection.Listed -> {
            item(key = "plan/pages") { PlanHeader("Pages", section.count, Modifier.testTag("plan-pages")) }
            for (page in section.pages) {
                item(key = "plan/page/${page.slot}") { PlanPageRow(page, hook.now) { onOpen(PlanPage.Page(page.slot)) } }
            }
        }
    }
}

/** One page's row: its title, its one-line summary and when it was updated. */
@Composable
fun PlanPageRow(page: BoardPage, now: Long, onClick: () -> Unit) {
    ListItem(
        leadingContent = {
            Icon(Icons.AutoMirrored.Outlined.Article, contentDescription = null, modifier = Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        },
        headlineContent = { Text(page.title, style = MaterialTheme.typography.titleSmall, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            Column {
                if (page.summary.isNotEmpty()) Text(page.summary, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(PageWords.updated(page, now), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.outline)
            }
        },
        trailingContent = { Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null, tint = MaterialTheme.colorScheme.outline) },
        modifier = Modifier
            .clickable(role = Role.Button, onClick = onClick)
            .testTag("plan-page-${page.slot}")
            .semantics(mergeDescendants = true) { contentDescription = PageSpeech.row(page, now) },
    )
}

/** A page pushed: its title, when it was written and by whom, then its blocks, in one scrolling column. */
@Composable
fun OrchestratorPage(page: BoardPage, world: PageWorld, onDestination: (PageDestination) -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp).testTag("plan-orchestrator-page")) {
        PageView(page, world, onDestination)
    }
}

/** The pages anchored to a theme, as sections of its page: each headed by its title, with "From the orchestrator · 2 h ago" opening it whole. */
fun LazyListScope.anchoredPageItems(pages: List<BoardPage>, world: PageWorld, onOpenPage: (PlanPage) -> Unit, onDestination: (PageDestination) -> Unit) {
    for (page in pages) {
        item(key = "anchored/${page.slot}") {
            Column(Modifier.testTag("plan-anchored-${page.slot}")) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 20.dp, bottom = 4.dp),
                ) {
                    Text(
                        page.title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.weight(1f).semantics { heading() },
                    )
                    TextButton(
                        onClick = { onOpenPage(PlanPage.Page(page.slot)) },
                        modifier = Modifier.testTag("plan-anchored-open-${page.slot}").semantics {
                            contentDescription = "Open ${page.title}, ${PageWords.updated(page, world.nowMs)}"
                        },
                    ) {
                        Text("${PageWords.FROM_THE_ORCHESTRATOR} · ${PlanWords.ago(page.updatedAtMs, world.nowMs)}", style = MaterialTheme.typography.labelMedium)
                        Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null, modifier = Modifier.padding(start = 2.dp).size(16.dp))
                    }
                }
                Box(Modifier.padding(horizontal = 8.dp)) {
                    val doc = page.doc
                    if (doc != null) PageBlocks(doc.under(page.title), world, onDestination)
                    else Text(PageWords.UNREADABLE, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(8.dp))
                }
            }
        }
    }
}

/** What a page's references are drawn from on this phone: the board's cards, its plan, its pages, and the fleet's worktrees and terminals by name. */
fun pageWorld(rows: List<TaskRow>, plan: Plan?, pages: List<BoardPage>, fleet: Fleet, now: Long = System.currentTimeMillis()): PageWorld {
    val names = mutableMapOf<String, String>()
    val terminals = mutableSetOf<String>()
    for (worktree in fleet.worktrees) {
        val path = worktree.worktree?.substringAfterLast('/').orEmpty()
        for (name in listOf(worktree.task, worktree.branch, path)) if (name.isNotEmpty() && name !in names) names[name] = worktree.id
        for (terminal in worktree.terminals) terminals += PageWorld.terminalKey(worktree.id, terminal.title)
    }
    return PageWorld(rows, plan, pages, names, terminals, now)
}

/**
 * Where a reference goes on Android. A question still waiting goes to Needs
 * You, where it's answered (design 3.4, Q3); one answered opens its task,
 * which [PageWorld.resolve] already chose. A worktree or a terminal opens its
 * pane over the page. A web link never reaches here: `PageBlocks` checks it
 * again and hands it to the browser.
 */
class PageRouter(
    private val hostId: String,
    private val fleet: Fleet,
    private val onOpenTask: (String) -> Unit,
    private val onOpenPage: (PlanPage) -> Unit,
    private val onNeedsYou: () -> Unit,
    private val onOpenTerminal: (TerminalRef) -> Unit,
) {
    fun open(destination: PageDestination) {
        when (destination) {
            is PageDestination.Task -> onOpenTask(destination.id)
            is PageDestination.Ask -> onNeedsYou()
            is PageDestination.Lane -> onOpenPage(PlanPage.Lane(destination.id))
            is PageDestination.Theme -> onOpenPage(PlanPage.Theme(destination.id))
            is PageDestination.Page -> onOpenPage(PlanPage.Page(destination.slot))
            is PageDestination.Worktree -> fleet.worktrees.firstOrNull { it.id == destination.id }?.terminals?.firstOrNull()?.let {
                onOpenTerminal(TerminalRef(hostId, destination.id, it.id))
            }
            is PageDestination.Terminal -> fleet.worktrees.firstOrNull { it.id == destination.worktree }?.terminals
                ?.firstOrNull { it.title == destination.name }?.let { onOpenTerminal(TerminalRef(hostId, destination.worktree, it.id)) }
            is PageDestination.Url -> Unit
        }
    }
}

/** The body of a page pushed, or why there's none. */
@Composable
fun OrchestratorPageBody(slot: String, state: PageListState?, world: PageWorld, onDestination: (PageDestination) -> Unit, onRetry: () -> Unit) {
    val page = (state as? PageListState.Loaded)?.pages?.firstOrNull { it.slot == slot }
    when {
        page != null -> OrchestratorPage(page, world, onDestination)
        state == PageListState.Unavailable -> EmptyState(PageWords.COULDNT_READ, "", Modifier.testTag("plan-pages-unavailable")) {
            TextButton(onClick = onRetry, modifier = Modifier.testTag("plan-pages-retry")) { Text(PlanWords.TRY_AGAIN) }
        }
        state == null -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { androidx.compose.material3.CircularProgressIndicator() }
        // Removed while it was open.
        else -> EmptyState("Page not found", "", Modifier.fillMaxSize().testTag("plan-page-gone"))
    }
}
