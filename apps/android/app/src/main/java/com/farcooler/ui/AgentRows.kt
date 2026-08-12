package com.farcooler.ui

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.ContentCut
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.PanTool
import androidx.compose.material.icons.outlined.Circle
import androidx.compose.material.icons.outlined.DonutLarge
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.Diff
import com.farcooler.model.DiffComputation
import com.farcooler.model.GapReason
import com.farcooler.model.PendingPermission
import com.farcooler.model.PermissionOption
import com.farcooler.model.PlanEntry
import com.farcooler.model.PlanStatus
import com.farcooler.model.Role
import com.farcooler.model.SubagentBlock
import com.farcooler.model.ToolRow
import com.farcooler.model.ToolStatus
import com.farcooler.model.TranscriptRow
import com.farcooler.model.active
import com.farcooler.model.doneCount

/**
 * One row of a rendered agent transcript.
 *
 * A thin switch, deliberately: the transcript already decided what happened —
 * coalesced message chunks, mutated a tool call in place rather than appending,
 * kept a gap as its own row — this only decides how each of the four shapes it
 * can hand back gets drawn.
 */
@Composable
fun AgentRowView(
    row: TranscriptRow,
    isLast: Boolean = false,
    pending: PendingPermission? = null,
    onAnswer: ((String) -> Unit)? = null,
) {
    when (val kind = row.kind) {
        // The parent pointer is deliberately ignored here. It exists so the
        // reducer can refuse to coalesce an orphan into the agent's own words;
        // once a message has been placed, where it came from changes nothing
        // about how it is drawn.
        is TranscriptRow.Kind.Message -> MessageRow(kind.role, kind.text, isLast)
        is TranscriptRow.Kind.Tool -> ToolRowView(kind.tool, isLast, pending, onAnswer)
        is TranscriptRow.Kind.Subagent -> SubagentBlockView(kind.block, pending, onAnswer)
        is TranscriptRow.Kind.Gap -> GapRow(kind.reason)
    }
}

/**
 * One message. Three shapes for three roles, because they answer three
 * different questions: what did I say, what did it say, and what did it think
 * before saying that.
 */
@Composable
private fun MessageRow(role: Role, text: String, isLive: Boolean) {
    when (role) {
        // Right-aligned with a fill — the one voice in the transcript that is
        // not the agent talking, and it has to read as a different speaker at a
        // glance, not on close reading.
        Role.USER -> Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            Spacer(Modifier.width(40.dp))
            Text(
                text,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier
                    .clip(RoundedCornerShape(14.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainerHighest)
                    .padding(horizontal = 12.dp, vertical = 8.dp),
            )
        }

        Role.AGENT -> Row(Modifier.fillMaxWidth()) {
            MarkdownText(text, modifier = Modifier.weight(1f))
            Spacer(Modifier.width(40.dp))
        }

        // Open while it is being written, closed once it is done. It was
        // collapsed always on iOS, on the reasoning that a finished thought is
        // scratch work — true, and it left a phone watching a long turn with
        // one word on screen and no sign of movement, which reads as stuck.
        Role.THOUGHT -> ThoughtRow(text, isLive)
    }
}

@Composable
private fun ThoughtRow(text: String, isLive: Boolean) {
    var expanded by remember { mutableStateOf(false) }
    // Open while live unless the reader has closed it; closed after unless the
    // reader has opened it.
    val showing = expanded || isLive

    Column {
        Row(
            Modifier.clickable { expanded = !expanded },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Chevron(showing)
            Spacer(Modifier.width(4.dp))
            Text(
                if (isLive) "Thinking…" else "Thought",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (showing) {
            // While it is being written, only the last few lines — enough to
            // see it moving, which is the whole point. One collapsed word looks
            // stuck; the whole thing pushes the conversation off the screen.
            MarkdownText(
                if (isLive && !expanded) tail(text) else text,
                secondary = true,
                modifier = Modifier.padding(top = 4.dp, start = 18.dp),
            )
        }
    }
}

private fun tail(text: String, lines: Int = 5): String {
    val all = text.split("\n")
    if (all.size <= lines) return text
    return all.takeLast(lines).joinToString("\n")
}

/**
 * One tool call, mutated in place by every update — never a new row — so a call
 * that reports progress four times still occupies the one line it earned.
 */
@Composable
private fun ToolRowView(
    tool: ToolRow,
    isLive: Boolean,
    pending: PendingPermission?,
    onAnswer: ((String) -> Unit)?,
) {
    var expanded by remember { mutableStateOf(false) }
    val expandable = tool.content != null || tool.diff != null
    val running = tool.status == ToolStatus.PENDING || tool.status == ToolStatus.IN_PROGRESS

    // Open while it is waiting to be approved, and while it is the thing
    // currently happening. Being asked to allow a command without being shown
    // it is a guess, not a decision. And a command running with nothing after
    // it IS the turn, so it opens itself and folds away once the agent moves on
    // — a transcript of every command's full output is unreadable.
    val showingDetail = expanded || pending != null || (isLive && running)

    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .then(
                if (pending != null) {
                    Modifier.border(
                        1.dp,
                        MaterialTheme.colorScheme.tertiary.copy(alpha = 0.55f),
                        RoundedCornerShape(8.dp),
                    )
                } else {
                    Modifier
                }
            )
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .then(if (expandable) Modifier.clickable { expanded = !expanded } else Modifier)
                .padding(horizontal = 9.dp, vertical = 7.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (expandable) {
                Chevron(expanded)
                Spacer(Modifier.width(5.dp))
            }
            StatusDot(toolStatusColor(tool.status))
            Spacer(Modifier.width(7.dp))
            Text(
                tool.title,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f, fill = false),
            )
            tool.locations.firstOrNull()?.let { location ->
                Spacer(Modifier.width(6.dp))
                Text(
                    location,
                    style = MaterialTheme.typography.labelSmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }

        if (showingDetail) {
            HorizontalDivider()
            Column(Modifier.padding(9.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                tool.content?.takeIf { it.isNotEmpty() }?.let { DetailBox(it) }
                tool.diff?.let { DiffView(it) }
            }
        }

        // The question, on the thing being asked about. No heading and no
        // coloured panel: the ring around this row already says which call is
        // waiting, and repeating it in words inside the row it is drawn on is
        // the same fact twice.
        if (pending != null && onAnswer != null) {
            HorizontalDivider()
            ApprovalControls(pending.options, onAnswer, Modifier.padding(9.dp))
        }
    }
}

/**
 * A tool's raw output, bounded.
 *
 * A tool that returns thousands of lines inside an expanding row froze the Mac
 * app when it was drawn whole, and a phone has less to spend. So the text is
 * clamped and says how much it clamped.
 */
@Composable
private fun DetailBox(text: String, maxLines: Int = 24) {
    val lines = remember(text) { text.split("\n") }
    val shown = remember(lines) { lines.take(maxLines).joinToString("\n") }
    Column {
        Box(
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(6.dp))
                .background(MaterialTheme.colorScheme.surfaceContainerHighest)
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 8.dp, vertical = 6.dp)
        ) {
            Text(
                shown,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
            )
        }
        if (lines.size > maxLines) {
            Text(
                "… ${lines.size - maxLines} more lines",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 2.dp),
            )
        }
    }
}

/**
 * A subagent's dispatch, and everything it did, as one object.
 *
 * The row and what it opens are one fill, so an expanded block cannot drift to
 * a different edge than the header that opened it. Its children are ordinary
 * rows — a subagent's messages and tools are the same things the top level
 * shows, and nesting is where they live rather than what they are.
 */
@Composable
private fun SubagentBlockView(
    block: SubagentBlock,
    pending: PendingPermission?,
    onAnswer: ((String) -> Unit)?,
) {
    // Null means nobody has said, so the automatic rule applies. Once a reader
    // touches it they win permanently: a block that shut itself while someone
    // was reading it is worse than one left open.
    var toggled by remember { mutableStateOf<Boolean?>(null) }
    var showingAll by remember { mutableStateOf(false) }

    val showing = pending != null || (toggled ?: block.isRunning)

    // The last few children rather than the first few: what a subagent is doing
    // now is what a reader is watching for, and the cap is what bounds a
    // block's height whether it holds three rows or three hundred.
    val shown =
        if (pending == null && !showingAll && block.children.size > VISIBLE_CHILDREN) {
            block.children.takeLast(VISIBLE_CHILDREN)
        } else {
            block.children
        }
    val hidden = block.children.size - shown.size

    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .clickable { toggled = !showing }
                .padding(horizontal = 9.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Chevron(showing)
            Spacer(Modifier.width(5.dp))
            // Red for interrupted, overriding the tool's own status on purpose:
            // a block cut off mid-flight is still IN_PROGRESS on the wire, and
            // a subagent whose outcome nobody knows must never wear the mark of
            // one that came back.
            StatusDot(
                if (block.interrupted) MaterialTheme.colorScheme.error
                else toolStatusColor(block.tool.status)
            )
            Spacer(Modifier.width(7.dp))
            Text(
                block.tool.title,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
                maxLines = 1,
                // Truncated in the middle, because a dispatch's title is the
                // prompt it was given and the end of that sentence says more
                // about what it went off to do than the middle does.
                overflow = TextOverflow.MiddleEllipsis,
                modifier = Modifier.weight(1f),
            )
            if (block.subtitle.isNotEmpty()) {
                Spacer(Modifier.width(6.dp))
                Text(
                    block.subtitle,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                )
            }
        }

        if (showing && block.children.isNotEmpty()) {
            HorizontalDivider()
            Column(
                Modifier.padding(9.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                if (hidden > 0) {
                    Text(
                        "… $hidden more",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.clickable { showingAll = true },
                    )
                }
                for (child in shown) {
                    // The approval controls are drawn by the child that is
                    // actually blocked, not by this block, so what is being
                    // approved and what will run stay the same object.
                    AgentRowView(
                        row = child,
                        pending = pending?.takeIf { names(it, child) },
                        onAnswer = onAnswer,
                    )
                }
            }
        }
    }
}

private const val VISIBLE_CHILDREN = 3

/**
 * Whether a row IS the tool call a request is asking about.
 *
 * Free rather than a method, because the same question is asked at two depths —
 * of a top-level row and of a block's child — and the two answers drifting
 * apart is how a request ends up claimed by nobody or by two views at once.
 */
fun names(pending: PendingPermission, row: TranscriptRow): Boolean =
    (row.kind as? TranscriptRow.Kind.Tool)?.tool?.id == pending.toolCall

/**
 * A break in the transcript, named rather than hidden.
 *
 * The one row here that is not allowed to be quiet. A gap is the opposite of
 * nothing — it is the transcript admitting history is missing — and drawing it
 * as a thin rule between two messages would let a reader miss the one fact this
 * whole design exists to never hide.
 */
@Composable
private fun GapRow(reason: GapReason) {
    // LoadEmpty is news, not a failure — a fresh chat pane has nothing to
    // restore because nothing happened yet, and the same "something broke"
    // language a real gap gets would tell the user the opposite of the truth.
    // It still gets a row: "nothing was lost" is exactly what this row exists
    // to say plainly rather than leave the user to infer from silence.
    val informational = reason is GapReason.LoadEmpty
    val tint =
        if (informational) MaterialTheme.colorScheme.onSurfaceVariant
        else MaterialTheme.colorScheme.tertiary

    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .background(tint.copy(alpha = 0.12f))
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (informational) Icons.Filled.Info else Icons.Filled.ContentCut,
            contentDescription = null,
            tint = tint,
            modifier = Modifier.size(16.dp),
        )
        Spacer(Modifier.width(8.dp))
        Text(
            when (reason) {
                GapReason.RingTrimmed -> "Some earlier history was trimmed and is not shown here."
                GapReason.LoadUnsupported -> "This session could not be loaded from where it left off."
                GapReason.LoadEmpty ->
                    "This session has no recorded turns yet — there is nothing to restore."
                is GapReason.LoadFailed ->
                    "This session could not be loaded from where it left off: ${reason.detail}"
                GapReason.Unparsed -> "Something happened here that this version cannot show."
            },
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.Medium,
            color = tint,
        )
    }
}

/**
 * The answers to a permission request.
 *
 * Ordered by what the question actually is. ACP hands back a flat list —
 * `allow_once`, `allow_always`, `reject_once`, … — and rendering it flat gives
 * three identical full-width buttons, two of them the same colour, with the
 * longest and loudest being a restatement of the command already shown above. A
 * stack of equal-weight options is not a decision; it is a menu.
 *
 * So: the decision is one row, Allow prominent and Reject plain beside it, and
 * everything else — the "always" variants, which are a policy change rather
 * than an answer to this question — sits under it in small type.
 *
 * Reject is NOT red. Red is for destructive; declining a command destroys
 * nothing, and spending the alarm colour here leaves none for when it matters.
 */
@Composable
fun ApprovalControls(
    options: List<PermissionOption>,
    onChoose: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val allow = options.firstOrNull { it.kind.lowercase().contains("once") && isAllow(it) }
        ?: options.firstOrNull(::isAllow)
    val reject = options.firstOrNull(::isReject)
    val secondary = options.filter { it.id != allow?.id && it.id != reject?.id }

    Column(modifier, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (allow != null) {
                Button(onClick = { onChoose(allow.id) }) { Text(allow.name) }
            }
            if (reject != null) {
                OutlinedButton(onClick = { onChoose(reject.id) }) { Text(reject.name) }
            }
        }
        // Kept, because an adapter may offer options this client has never
        // heard of and swallowing them would make an answer unreachable.
        for (option in secondary) {
            Text(
                option.name,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
                modifier = Modifier.clickable { onChoose(option.id) },
            )
        }
    }
}

private fun isAllow(option: PermissionOption): Boolean {
    val kind = option.kind.lowercase()
    return kind.contains("allow") || kind.contains("accept")
}

private fun isReject(option: PermissionOption): Boolean {
    val kind = option.kind.lowercase()
    return kind.contains("reject") || kind.contains("deny")
}

/** A request naming a tool call the transcript has no row for. */
@Composable
fun ApprovalCard(pending: PendingPermission, onChoose: (String) -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(MaterialTheme.colorScheme.tertiaryContainer)
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                Icons.Filled.PanTool,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onTertiaryContainer,
                modifier = Modifier.size(16.dp),
            )
            Spacer(Modifier.width(8.dp))
            Text(
                "Needs your approval",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onTertiaryContainer,
            )
        }
        ApprovalControls(pending.options, onChoose)
    }
}

/**
 * The agent's task list, as the agent maintains it.
 *
 * A list of bullets is the same information and none of the use — what a reader
 * wants is how far through it is and what is happening right now.
 */
@Composable
fun PlanPanel(entries: List<PlanEntry>) {
    var expanded by remember { mutableStateOf(true) }
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(14.dp))
            // OPAQUE, because it floats over a scrolling transcript. A tinted
            // overlay let the conversation through, and expanding the list
            // turned both into one unreadable overlap.
            .background(MaterialTheme.colorScheme.surfaceContainerHigh)
            .padding(horizontal = 14.dp, vertical = 10.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(
            Modifier.fillMaxWidth().clickable { expanded = !expanded },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Chevron(expanded)
            Spacer(Modifier.width(6.dp))
            Text("Tasks", style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.width(6.dp))
            Text(
                "${entries.doneCount} of ${entries.size}",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (!expanded) {
                entries.active?.let { active ->
                    Spacer(Modifier.width(6.dp))
                    Text(
                        "· ${active.content}",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
        if (expanded) {
            for (entry in entries) {
                val status = PlanStatus.parse(entry.status)
                Row(verticalAlignment = Alignment.Top) {
                    Icon(
                        when (status) {
                            PlanStatus.DONE -> Icons.Filled.CheckCircle
                            PlanStatus.ACTIVE -> Icons.Outlined.DonutLarge
                            PlanStatus.PENDING -> Icons.Outlined.Circle
                        },
                        contentDescription = null,
                        tint = when (status) {
                            PlanStatus.DONE -> Color(0xFF4CAF50)
                            PlanStatus.ACTIVE -> MaterialTheme.colorScheme.primary
                            PlanStatus.PENDING -> MaterialTheme.colorScheme.onSurfaceVariant
                        },
                        modifier = Modifier.size(15.dp).padding(top = 1.dp),
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        entry.content,
                        style = MaterialTheme.typography.bodySmall,
                        color =
                            if (status.isDone) MaterialTheme.colorScheme.onSurfaceVariant
                            else MaterialTheme.colorScheme.onSurface,
                        textDecoration =
                            if (status.isDone) androidx.compose.ui.text.style.TextDecoration.LineThrough
                            else null,
                    )
                }
            }
        }
    }
}

/**
 * A unified diff, computed client-side from the two full texts a tool call
 * carries.
 *
 * No syntax highlighting. What earns the pixels here is which lines changed,
 * not what language they are in.
 */
@Composable
fun DiffView(diff: Diff) {
    val lines = remember(diff) { DiffComputation.compute(diff.oldText.orEmpty(), diff.newText) }
    var expanded by remember { mutableStateOf(false) }

    val added = lines.count { it.kind == DiffComputation.Kind.ADDED }
    val removed = lines.count { it.kind == DiffComputation.Kind.REMOVED }

    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                diff.path,
                style = MaterialTheme.typography.labelSmall,
                fontFamily = FontFamily.Monospace,
                fontWeight = FontWeight.Medium,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
                modifier = Modifier.weight(1f),
            )
            if (added > 0) {
                Text(
                    "+$added",
                    style = MaterialTheme.typography.labelSmall,
                    fontFamily = FontFamily.Monospace,
                    color = Color(0xFF4CAF50),
                )
                Spacer(Modifier.width(6.dp))
            }
            if (removed > 0) {
                Text(
                    "-$removed",
                    style = MaterialTheme.typography.labelSmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.error,
                )
            }
        }

        // Beyond this many lines the diff opens collapsed. A four-line edit is
        // worth seeing on arrival; a four-hundred-line rewrite is not something
        // to scroll past to reach the message after it — doubly so on a screen
        // this narrow.
        if (lines.size > COLLAPSE_THRESHOLD && !expanded) {
            Text(
                "Show ${lines.size} lines",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.clickable { expanded = true },
            )
        } else {
            Column(
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(6.dp))
                    .background(MaterialTheme.colorScheme.surfaceContainerHighest)
                    .horizontalScroll(rememberScrollState())
                    .padding(vertical = 4.dp)
            ) {
                for (line in lines) {
                    Row(
                        Modifier
                            .background(
                                when (line.kind) {
                                    DiffComputation.Kind.ADDED -> Color(0x264CAF50)
                                    DiffComputation.Kind.REMOVED -> Color(0x26F44336)
                                    DiffComputation.Kind.CONTEXT -> Color.Transparent
                                }
                            )
                            .padding(horizontal = 4.dp, vertical = 1.dp)
                    ) {
                        Gutter(line.oldNumber)
                        Gutter(line.newNumber)
                        Text(
                            when (line.kind) {
                                DiffComputation.Kind.ADDED -> "+"
                                DiffComputation.Kind.REMOVED -> "-"
                                DiffComputation.Kind.CONTEXT -> " "
                            },
                            style = MaterialTheme.typography.labelSmall,
                            fontFamily = FontFamily.Monospace,
                            modifier = Modifier.width(12.dp),
                        )
                        Text(
                            line.text.ifEmpty { " " },
                            style = MaterialTheme.typography.labelSmall,
                            fontFamily = FontFamily.Monospace,
                            color =
                                if (line.kind == DiffComputation.Kind.CONTEXT)
                                    MaterialTheme.colorScheme.onSurfaceVariant
                                else MaterialTheme.colorScheme.onSurface,
                        )
                    }
                }
            }
        }
    }
}

private const val COLLAPSE_THRESHOLD = 20

@Composable
private fun Gutter(number: Int?) {
    Text(
        number?.toString().orEmpty(),
        style = MaterialTheme.typography.labelSmall,
        fontFamily = FontFamily.Monospace,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        textAlign = androidx.compose.ui.text.style.TextAlign.End,
        modifier = Modifier.width(26.dp),
    )
}

/**
 * The vocabulary for "something is happening": green finished, red missing,
 * secondary for everything in between. The attention colour is reserved for
 * "needs you", which a tool call never is.
 */
@Composable
fun toolStatusColor(status: ToolStatus): Color = when (status) {
    ToolStatus.PENDING -> MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.35f)
    ToolStatus.IN_PROGRESS -> MaterialTheme.colorScheme.onSurfaceVariant
    ToolStatus.COMPLETED -> Color(0xFF4CAF50)
    ToolStatus.FAILED -> MaterialTheme.colorScheme.error
    // A status from a newer daemon. Neutral on purpose: it finished, and
    // claiming either success or failure would be inventing a detail.
    ToolStatus.UNKNOWN -> MaterialTheme.colorScheme.onSurfaceVariant
}

@Composable
private fun StatusDot(color: Color) {
    Box(Modifier.size(7.dp).clip(CircleShape).background(color))
}

@Composable
private fun Chevron(open: Boolean) {
    val angle by animateFloatAsState(if (open) 90f else 0f, label = "chevron")
    Icon(
        Icons.AutoMirrored.Filled.KeyboardArrowRight,
        contentDescription = null,
        tint = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.size(16.dp).rotate(angle),
    )
}
