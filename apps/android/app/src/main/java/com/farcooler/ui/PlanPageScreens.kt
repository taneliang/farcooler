package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.model.GlancePalette
import com.farcooler.model.Plan
import com.farcooler.model.PlanCardRef
import com.farcooler.model.PlanLane
import com.farcooler.model.PlanPage
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanRecord
import com.farcooler.model.PlanTheme
import com.farcooler.model.PlanWords
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskUsageFormat
import com.farcooler.net.Connection
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.launch

// A theme's page and a lane's page (ov-268 design 6.2, 6.3 and 6.5), pushed
// from the Plan view, in one column: the same sections as the Mac's pages. A
// card on either opens its task, as a card on the board does.

/** The page for [page], or why there's none. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PlanPageScreen(
    connection: Connection,
    workspaceId: String,
    page: PlanPage,
    onOpenTask: (taskId: String) -> Unit,
    onOpenPage: (PlanPage) -> Unit,
    onBack: () -> Unit,
) {
    val states by connection.plans.states.collectAsStateWithLifecycle()
    val records by connection.plans.records.collectAsStateWithLifecycle()
    val boards by connection.boards.collectAsStateWithLifecycle()
    val workspace = connection.board(workspaceId)
    val state = states[workspaceId]
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    LaunchedEffect(page) {
        if (states[workspaceId] !is PlanReadState.Loaded) connection.plans.read(workspace)
        connection.plans.readRecord(page)
    }
    val plan = (state as? PlanReadState.Loaded)?.plan
    val title = when (page) {
        is PlanPage.Theme -> plan?.themes?.firstOrNull { it.id == page.id }?.name ?: "Theme"
        is PlanPage.Lane -> plan?.lanes?.firstOrNull { it.id == page.id }?.name ?: "Lane"
    }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") }
                },
            )
        },
    ) { padding ->
        Box(Modifier.padding(padding).fillMaxSize()) {
            when (state) {
                null, PlanReadState.Loading -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                PlanReadState.NeedsUpdate -> EmptyState(PlanWords.NEEDS_UPDATE, "", Modifier.testTag("plan-needs-update"))
                PlanReadState.Unavailable -> EmptyState(
                    PlanWords.COULDNT_READ, "", Modifier.testTag("plan-unavailable"),
                    icon = Icons.Outlined.Warning,
                ) {
                    TextButton(onClick = { scope.launch { connection.plans.read(workspace) } }, modifier = Modifier.testTag("plan-retry")) {
                        Text(PlanWords.TRY_AGAIN)
                    }
                }
                is PlanReadState.Loaded -> PlanPageBody(
                    plan = state.plan,
                    page = page,
                    record = records[page],
                    rows = boards[workspaceId]?.rows.orEmpty().associateBy { it.id },
                    onOpenTask = onOpenTask,
                    onOpenPage = onOpenPage,
                )
            }
        }
    }
}

/** The body of a plan page, from a plan read: what the screen and the captures draw. */
@Composable
fun PlanPageBody(
    plan: Plan,
    page: PlanPage,
    record: PlanRecord?,
    rows: Map<String, TaskRow>,
    onOpenTask: (String) -> Unit,
    onOpenPage: (PlanPage) -> Unit,
) {
    when (page) {
        is PlanPage.Theme -> {
            val theme = plan.themes.firstOrNull { it.id == page.id }
            if (theme == null) EmptyState("Theme not found", "", Modifier.fillMaxSize())
            else PlanThemePage(theme, plan, record, rows, onOpenTask, onOpenPage)
        }
        is PlanPage.Lane -> {
            val lane = plan.lanes.firstOrNull { it.id == page.id }
            if (lane == null) EmptyState("Lane not found", "", Modifier.fillMaxSize())
            else PlanLanePage(lane, plan, record, rows, onOpenTask, onOpenPage)
        }
    }
}

@Composable
private fun PlanSectionTitle(title: String, trailing: String? = null) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 4.dp),
    ) {
        Text(title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary, modifier = Modifier.weight(1f))
        if (trailing != null) Text(trailing, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun Body(text: String, modifier: Modifier = Modifier, muted: Boolean = false) {
    Text(
        text,
        style = MaterialTheme.typography.bodyLarge,
        color = if (muted) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
        modifier = modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
    )
}

/** A theme's page: its outcome, where it stands and what changed, what's next and what needs you, its lanes and cards. */
@Composable
fun PlanThemePage(
    theme: PlanTheme,
    plan: Plan,
    record: PlanRecord?,
    rows: Map<String, TaskRow>,
    onOpenTask: (String) -> Unit,
    onOpenPage: (PlanPage) -> Unit,
) {
    var showingChange by rememberSaveable(theme.id) { mutableStateOf(false) }
    val before = record?.previousStory
    LazyColumn(Modifier.fillMaxSize().testTag("plan-theme-page")) {
        item {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(theme.name, style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                    Text(theme.state.replaceFirstChar { it.uppercase() }, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (theme.outcome.isNotEmpty()) Text(theme.outcome, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                PlanProgressBar(theme.counts)
                Text(PlanWords.progress(theme.counts), style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"), color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        item {
            PlanSectionTitle("Where it stands", theme.storyAt.takeIf { it > 0 }?.let { "Updated ${PlanWords.ago(it, plan.nowMs)}" })
            if (theme.story.isEmpty()) Body("The orchestrator hasn’t written where this stands yet.", muted = true) else Body(theme.story)
            if (showingChange && before != null) {
                Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("plan-previous-story")) {
                    Text("Before, ${PlanWords.ago(before.second, plan.nowMs)}", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text(before.first, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            if (before != null) {
                TextButton(onClick = { showingChange = !showingChange }, contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp), modifier = Modifier.testTag("plan-what-changed")) {
                    Text(if (showingChange) "Hide changes" else "What changed")
                }
            }
        }
        if (theme.next.isNotEmpty() || theme.ownerAsk.isNotEmpty()) {
            item {
                Column(Modifier.padding(horizontal = 16.dp)) {
                    if (theme.next.isNotEmpty()) {
                        SectionTitleInline("Next")
                        Text(theme.next, style = MaterialTheme.typography.bodyLarge)
                    }
                    if (theme.ownerAsk.isNotEmpty()) PlanAsk(theme.ownerAsk, Modifier.padding(top = 8.dp))
                }
            }
        }
        val lanes = plan.lanesIn(theme)
        if (lanes.isNotEmpty()) {
            item { PlanSectionTitle("Lanes") }
            lanes.forEach { lane ->
                item(key = "lane/${lane.id}") {
                    PageLaneRow(lane) { onOpenPage(PlanPage.Lane(lane.id)) }
                }
            }
        }
        item { PlanSectionTitle("Cards", PlanWords.breakdown(theme.counts)) }
        cardRows(theme.cards, plan, rows, onOpenTask)
    }
}

@Composable
private fun SectionTitleInline(title: String) {
    Text(title, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 12.dp))
}

/** A lane on a theme's page: its name, its state, and its cards' keys. */
@Composable
private fun PageLaneRow(lane: PlanLane, onClick: () -> Unit) {
    ListItem(
        leadingContent = { Icon(planGlyph(lane.state), contentDescription = null, modifier = Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) },
        headlineContent = { Text(lane.name, style = MaterialTheme.typography.titleSmall) },
        supportingContent = {
            Column {
                Text(PlanWords.status(lane))
                Text(lane.cards.joinToString(" ") { it.key }, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.labelSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        },
        trailingContent = { Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null, tint = MaterialTheme.colorScheme.outline) },
        modifier = Modifier.clickable(onClick = onClick).testTag("plan-page-lane-${lane.name}").semantics(mergeDescendants = true) {},
    )
}

/** Cards as the board draws them, from the board's rows; a card the board hasn't read draws from the plan's read of it. */
private fun LazyListScope.cardRows(refs: List<PlanCardRef>, plan: Plan, rows: Map<String, TaskRow>, onOpenTask: (String) -> Unit) {
    refs.forEach { ref ->
        item(key = "card/${ref.task}/${ref.slice}") {
            val row = rows[ref.task]
            val card = plan.card(ref.task)
            val status = row?.status?.title ?: card?.status.orEmpty()
            ListItem(
                overlineContent = { Text(row?.key ?: ref.key, fontFamily = FontFamily.Monospace) },
                headlineContent = { Text(row?.title ?: card?.title.orEmpty(), maxLines = 3, overflow = TextOverflow.Ellipsis) },
                supportingContent = { Text(if (ref.slice.isEmpty()) status else "$status · ${ref.slice}") },
                modifier = (if (row != null) Modifier.clickable { onOpenTask(row.id) } else Modifier)
                    .testTag("plan-card-${row?.key ?: ref.key}").semantics(mergeDescendants = true) {},
            )
        }
    }
}

/** A lane's page: why it exists, where it runs, its cards, what it spent, its agents and its timeline. */
@Composable
fun PlanLanePage(
    lane: PlanLane,
    plan: Plan,
    record: PlanRecord?,
    rows: Map<String, TaskRow>,
    onOpenTask: (String) -> Unit,
    onOpenPage: (PlanPage) -> Unit,
) {
    val theme = plan.themeOf(lane)
    val place = listOf(
        lane.worktreePath, lane.branch.takeIf { it != lane.worktreePath }.orEmpty(), PlanWords.model(lane.model),
        lane.train?.let { "train $it" }.orEmpty(),
    ).filter { it.isNotEmpty() }.joinToString(" · ")
    LazyColumn(Modifier.fillMaxSize().testTag("plan-lane-page")) {
        item {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(lane.name, style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                    Text(PlanWords.status(lane), style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (lane.reason.isNotEmpty()) Text(lane.reason, style = MaterialTheme.typography.bodyLarge)
                if (place.isNotEmpty()) Text(place, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                PlanWords.stale(lane, plan.nowMs)?.let {
                    Text(it, style = MaterialTheme.typography.labelLarge, color = glanceColor(GlancePalette.amber))
                }
                if (theme != null) {
                    TextButton(onClick = { onOpenPage(PlanPage.Theme(theme.id)) }, contentPadding = androidx.compose.foundation.layout.PaddingValues(0.dp), modifier = Modifier.testTag("plan-lane-theme")) { Text(theme.name) }
                }
            }
        }
        item { PlanSectionTitle("Cards") }
        cardRows(lane.cards, plan, rows, onOpenTask)
        item {
            PlanSectionTitle("Spend")
            Column(Modifier.padding(horizontal = 16.dp).testTag("plan-lane-spend")) {
                Text("${PlanWords.spend(lane.spend)} · ${PlanWords.fixRounds(lane.fixRounds)}", style = MaterialTheme.typography.bodyLarge)
                if (lane.spend.totalTokens > 0 && (lane.spend.costMicros ?: 0) > 0) {
                    Text(TaskUsageFormat.API_EQUIVALENT, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
        if (lane.agents.isNotEmpty()) {
            item { PlanSectionTitle("Agents") }
            lane.agents.forEachIndexed { index, agent -> item(key = "agent/$index") { Body(PlanWords.agent(agent, plan.nowMs)) } }
        }
        val timeline = record?.timeline.orEmpty()
        if (timeline.isNotEmpty()) {
            item { PlanSectionTitle("Timeline") }
            timeline.forEachIndexed { index, row ->
                item(key = "timeline/$index") {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp).testTag("plan-timeline-row")) {
                        Text(
                            timelineTime(row.at, plan.nowMs),
                            style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.width(88.dp),
                        )
                        Text(row.text, style = MaterialTheme.typography.bodyMedium)
                    }
                }
            }
        }
    }
}

/** "15:02" on the runner's day; "Oct 3, 15:02" before. */
internal fun timelineTime(ms: Long, nowMs: Long): String {
    val date = Date(ms)
    val sameDay = DateFormat.getDateInstance(DateFormat.SHORT).format(date) == DateFormat.getDateInstance(DateFormat.SHORT).format(Date(nowMs))
    val time = DateFormat.getTimeInstance(DateFormat.SHORT).format(date)
    return if (sameDay) time else "${java.text.SimpleDateFormat("MMM d", java.util.Locale.getDefault()).format(date)}, $time"
}
