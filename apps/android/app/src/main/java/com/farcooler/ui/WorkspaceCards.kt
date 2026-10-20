package com.farcooler.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandHorizontally
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.farcooler.model.AgentOpening
import com.farcooler.model.GlanceMarkSize
import com.farcooler.model.GlancePalette
import com.farcooler.model.PlanStrip
import com.farcooler.model.RunningAgent
import com.farcooler.model.RunningAgents

// The two cards at the top of a workspace's conversation (ov-616, ov-619, ov-641), as the iPhone draws them: Now/Next,
// most of the width, and Agents beside it, compact. One tap target each, never two in one card.
//
// - Now/Next: the orchestrator's state as a mark, the band's Now and Next one line each, and what needs you as a dot
//   and a count. It opens the Plan as a sheet.
// - Agents: up to three status dots and how many agents run (Claude subagents and Far Cooler's own alike,
//   [RunningAgents]). It opens the agents view. With none running it goes, and Now/Next takes the row.
//
// At large font sizes the two stack, Agents under Now/Next, and Now/Next shows Now on two lines with Next left to the plan
// sheet: both on two lines took the screen. The iPhone does the same at its accessibility sizes.

/** The font scale from which the two cards stack. */
internal const val STACK_FONT_SCALE = 1.3f

@Composable
internal fun WorkspaceCards(
    strip: PlanStrip?,
    agents: RunningAgents,
    onOpenPlan: () -> Unit,
    onOpenAgents: () -> Unit,
    modifier: Modifier = Modifier,
    /** Draw Now/Next. A wide layout has the plan beside the chat, and carries the Agents card alone. */
    showsPlan: Boolean = true,
) {
    val stacked = LocalDensity.current.fontScale >= STACK_FONT_SCALE
    val running = agents.count > 0
    if (!showsPlan && !running) return
    val padding = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)
    if (stacked) {
        Column(modifier.then(padding).testTag("workspace-cards"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (showsPlan && strip != null) NowNextCard(strip, onOpenPlan, Modifier.fillMaxWidth())
            if (running) AgentsCard(agents, onOpenAgents, Modifier.fillMaxWidth(), wide = true)
        }
    } else {
        Row(
            modifier.then(padding).height(IntrinsicSize.Min).testTag("workspace-cards"),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (showsPlan && strip != null) NowNextCard(strip, onOpenPlan, Modifier.weight(1f).fillMaxHeight())
            AnimatedVisibility(
                visible = running,
                enter = fadeIn() + expandHorizontally(expandFrom = Alignment.End),
                exit = fadeOut() + shrinkHorizontally(shrinkTowards = Alignment.End),
            ) {
                AgentsCard(agents, onOpenAgents, if (showsPlan) Modifier.fillMaxHeight() else Modifier.fillMaxWidth(), wide = !showsPlan)
            }
        }
    }
}

private val CardShape = RoundedCornerShape(Radius.large)

/** Now and Next, on two lines, with what needs you and the chevron that says it opens. */
@Composable
internal fun NowNextCard(strip: PlanStrip, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val band = strip.band
    Surface(
        onClick = onClick,
        shape = CardShape,
        tonalElevation = 3.dp,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = modifier.testTag("plan-strip").semantics(mergeDescendants = true) {
            contentDescription = if (band != null) strip.bandAccessibilityLabel else "Plan"
        },
    ) {
        Row(
            Modifier.padding(horizontal = 14.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(stateIcon(strip.orchestrator), null, Modifier.size(18.dp), tint = tone(strip.orchestrator.tone))
            Spacer(Modifier.width(10.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                if (band != null) {
                    // At large font sizes Now takes two lines and Next is left to the sheet: both on two lines took the screen.
                    val large = LocalDensity.current.fontScale >= STACK_FONT_SCALE
                    BandLine("Now", band.nowLine, quiet = band.now == null, tag = "plan-band-now", lines = if (large) 2 else 1)
                    if (!large) BandLine("Next", band.nextLine, quiet = band.next == null, tag = "plan-band-next")
                } else if (strip.isEmpty) {
                    Text("Plan", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                } else {
                    StripWords(strip, Modifier)
                }
            }
            if (strip.needsYou > 0) {
                Spacer(Modifier.width(8.dp))
                NeedsYouBadge(strip.needsYou)
            }
            Spacer(Modifier.width(6.dp))
            Icon(Icons.Outlined.KeyboardArrowUp, null, tint = MaterialTheme.colorScheme.outline)
        }
    }
}

@Composable
private fun BandLine(label: String, words: String, quiet: Boolean, tag: String, lines: Int = 1) {
    // The label's column is 48 sp wide, not dp, so "NEXT" is whole at any font size.
    val column = with(LocalDensity.current) { 48.sp.toDp() }
    Row(verticalAlignment = Alignment.Top, modifier = Modifier.testTag(tag)) {
        Text(
            label.uppercase(),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.width(column).padding(top = 3.dp),
            maxLines = 1,
        )
        Text(
            words,
            style = MaterialTheme.typography.bodyMedium.copy(fontWeight = if (quiet) FontWeight.Normal else FontWeight.Medium),
            color = if (quiet) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
            maxLines = lines,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
    }
}

/** The count of what needs you, in an amber dot with the number: the dot carries the color and the number the meaning. */
@Composable
private fun NeedsYouBadge(count: Int) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.testTag("plan-strip-needs-you")) {
        Box(Modifier.size(7.dp).clip(CircleShape).background(glanceColor(GlancePalette.amber)))
        Spacer(Modifier.width(5.dp))
        Text("$count", style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.SemiBold))
    }
}

/** A short stack of status dots, overlapping as avatars do, and how many agents run. */
@Composable
internal fun AgentsCard(agents: RunningAgents, onClick: () -> Unit, modifier: Modifier = Modifier, wide: Boolean = false) {
    Surface(
        onClick = onClick,
        shape = CardShape,
        tonalElevation = 3.dp,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = modifier.testTag("agents-card").semantics(mergeDescendants = true) {
            contentDescription = agents.summary
            stateDescription = "${agents.count}"
            role = Role.Button
        },
    ) {
        Row(
            Modifier.padding(horizontal = 14.dp, vertical = 10.dp).heightIn(min = 24.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy((-2).dp)) {
                agents.dotAgents.forEach { AgentMark(it, GlanceMarkSize.RIBBON) }
            }
            // Stacked, the card has the row's width, and says it in words.
            Text(
                if (wide) agents.summary else "${agents.count}",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
            )
        }
    }
}

/** The agents view: a sheet from the Agents card, each row opening its agent. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun AgentsSheet(
    agents: RunningAgents,
    onChoose: (RunningAgent) -> Unit,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = false),
        modifier = Modifier.testTag("agents-sheet"),
    ) {
        AgentsSheetContent(agents, onChoose)
    }
}

/** What the sheet holds: its title, then the agents. Apart from the sheet, so a capture can draw the real thing. */
@Composable
internal fun AgentsSheetContent(agents: RunningAgents, onChoose: (RunningAgent) -> Unit) {
    Text(
        "Agents",
        style = MaterialTheme.typography.titleLarge,
        modifier = Modifier.padding(start = CONTENT_EDGE, end = CONTENT_EDGE, top = 4.dp, bottom = 4.dp).testTag("agents-title"),
    )
    AgentsList(agents, onChoose)
}

/** The edge content sits at, in the cards and the agents view: 16 dp, as the screen's own margin. */
internal val CONTENT_EDGE = 16.dp

/** What the agents view says. */
internal object AgentsWords {
    const val NONE_TITLE = "No agents running"
    const val NONE_BODY = "Agents the orchestrator starts, and the subagents they start, show up here while they work."
    const val ORCHESTRATOR_SECTION = "Orchestrator’s subagents"
    const val PANES_SECTION = "Agents"
}

/** The agents, as a list, the orchestrator's subagents first and under their own heading. */
@Composable
internal fun AgentsList(agents: RunningAgents, onChoose: (RunningAgent) -> Unit, modifier: Modifier = Modifier) {
    if (agents.agents.isEmpty()) {
        Column(modifier.fillMaxWidth().padding(CONTENT_EDGE).testTag("agents-none"), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(AgentsWords.NONE_TITLE, style = MaterialTheme.typography.titleMedium)
            Text(AgentsWords.NONE_BODY, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        return
    }
    val theirs = agents.agents.takeWhile { it.kind == RunningAgent.Kind.SUBAGENT }
    val rest = agents.agents.drop(theirs.size)
    LazyColumn(modifier.fillMaxWidth().testTag("agents-list"), contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 24.dp)) {
        if (theirs.isNotEmpty()) {
            item(key = "head-orchestrator") { SectionHeader(AgentsWords.ORCHESTRATOR_SECTION) }
            items(theirs, key = { it.id }) { AgentRowItem(it, nested = false, onChoose = onChoose) }
        }
        if (rest.isNotEmpty()) {
            if (theirs.isNotEmpty()) item(key = "head-panes") { SectionHeader(AgentsWords.PANES_SECTION) }
            items(rest, key = { it.id }) { AgentRowItem(it, nested = it.kind == RunningAgent.Kind.SUBAGENT, onChoose = onChoose) }
        }
    }
}

@Composable
private fun SectionHeader(title: String) {
    Text(
        title,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(start = CONTENT_EDGE, end = CONTENT_EDGE, top = 16.dp, bottom = 4.dp),
    )
}

/**
 * One agent: its dot, its name, its run's time and the chevron on one line, and what it's doing under the name. At large
 * font sizes the time goes under what it's doing, where it has the room. Every row is as tall as the tallest of the
 * ordinary ones, a time or none.
 */
@Composable
private fun AgentRowItem(agent: RunningAgent, nested: Boolean, onChoose: (RunningAgent) -> Unit) {
    val stacked = LocalDensity.current.fontScale >= STACK_FONT_SCALE
    val start = if (nested) CONTENT_EDGE + 24.dp else CONTENT_EDGE
    val rowTag = "agent-row-${agent.terminal}${if (agent.kind == RunningAgent.Kind.SUBAGENT) "-${agent.row ?: agent.name}" else ""}"
    Surface(
        onClick = { onChoose(agent) },
        color = androidx.compose.ui.graphics.Color.Transparent,
        modifier = Modifier.fillMaxWidth().testTag(rowTag).semantics(mergeDescendants = true) { role = Role.Button },
    ) {
        Column(
            Modifier.heightIn(min = 64.dp).padding(start = start, end = CONTENT_EDGE, top = 10.dp, bottom = 10.dp),
            verticalArrangement = Arrangement.Center,
        ) {
            Row(verticalAlignment = if (stacked) Alignment.Top else Alignment.CenterVertically) {
                Box(Modifier.padding(top = if (stacked) 8.dp else 0.dp).width(MARK_COLUMN), contentAlignment = Alignment.CenterStart) {
                    AgentMark(agent, GlanceMarkSize.ROW)
                }
                Text(
                    agent.name,
                    style = MaterialTheme.typography.bodyLarge.copy(
                        fontWeight = if (agent.kind == RunningAgent.Kind.ORCHESTRATOR) FontWeight.SemiBold else FontWeight.Medium,
                    ),
                    maxLines = if (stacked) 3 else 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f).testTag("$rowTag-name"),
                )
                if (!stacked && agent.startedMs != null) {
                    Spacer(Modifier.width(8.dp))
                    Box(Modifier.testTag("$rowTag-time")) { RunTimeChip(agent.startedMs, null) }
                }
                Icon(
                    Icons.AutoMirrored.Outlined.KeyboardArrowRight, null,
                    Modifier.padding(start = 4.dp).size(20.dp).testTag("$rowTag-go"), tint = MaterialTheme.colorScheme.outline,
                )
            }
            if (agent.activity.isNotEmpty()) {
                Text(
                    agent.activity,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(start = MARK_COLUMN),
                )
            }
            if (stacked && agent.startedMs != null) {
                Box(Modifier.padding(start = MARK_COLUMN, top = 2.dp)) { RunTimeChip(agent.startedMs, null) }
            }
        }
    }
    Separator(Modifier.padding(start = start, end = CONTENT_EDGE).testTag("$rowTag-rule"))
}

/** The column the mark has, so the name and what it's doing start at one edge. */
private val MARK_COLUMN = 22.dp

/** An agent's mark: its ring, red where its last turn died. */
@Composable
internal fun AgentMark(agent: RunningAgent, size: GlanceMarkSize) {
    GlanceMarkView(
        agent.mark, size, decorative = true,
        ringOverride = if (agent.failed) MaterialTheme.colorScheme.error else null,
    )
}

/**
 * Carry out where an agent went (ov-641): the subagent's own conversation is asked of its pane's model (the pane's, once it
 * is up, when it isn't yet), then the pane or the chat is brought to. [goTo] opens a pane by worktree and terminal;
 * [focusChat] puts the keyboard in the chat beside the plan.
 */
internal fun AgentOpening.carryOut(
    panes: com.farcooler.net.NativePanes,
    wide: Boolean,
    goTo: (worktree: String, terminal: String) -> Unit,
    focusChat: () -> Unit,
) {
    when (this) {
        AgentOpening.Stay -> Unit
        AgentOpening.FocusChat -> focusChat()
        is AgentOpening.OpenOwn -> {
            panes.open(terminal, row, agentId)
            if (pane != null) goTo(pane, terminal) else if (wide) focusChat()
        }
        is AgentOpening.OpenPane -> goTo(worktree, terminal)
    }
}

/** The wide layout's chat column head: the Agents card alone, the plan being beside it, and its sheet. */
@Composable
internal fun WideAgentsBar(agents: RunningAgents, onOpenAgent: (RunningAgent) -> Unit) {
    var viewing by androidx.compose.runtime.remember { androidx.compose.runtime.mutableStateOf(false) }
    WorkspaceCards(null, agents, onOpenPlan = {}, onOpenAgents = { viewing = true }, showsPlan = false)
    if (viewing) AgentsSheet(agents, onChoose = { viewing = false; onOpenAgent(it) }, onDismiss = { viewing = false })
}
