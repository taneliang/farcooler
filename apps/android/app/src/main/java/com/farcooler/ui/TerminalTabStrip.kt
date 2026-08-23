package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.StateKind
import com.farcooler.net.FleetEntry
import com.farcooler.net.TerminalRef

/**
 * Every terminal in the fleet, one tap from whichever one is on screen.
 *
 * The Mac always has its sidebar up, so switching terminals there is a click
 * away regardless of which is open. A phone's terminal screen is full-bleed,
 * and "open the drawer, find the row, tap it" is the wrong cost for something
 * as routine as glancing at a second agent. This makes every terminal one tap
 * away without ever leaving the screen that made checking on it worthwhile.
 *
 * Deliberately flat across the whole fleet — every runner included — rather
 * than scoped to the current workspace: the 3am case this exists for is "is the
 * OTHER agent still blocked", which is exactly as likely to be on a different
 * runner as in the same workspace.
 */
@Composable
fun TerminalTabStrip(
    entries: List<FleetEntry>,
    showRunner: Boolean,
    current: TerminalRef,
    onSelect: (TerminalRef) -> Unit,
    modifier: Modifier = Modifier,
) {
    data class Chip(
        val ref: TerminalRef,
        val label: String,
        val runner: String,
        val kind: StateKind,
        val wantsAttention: Boolean,
        val attention: com.farcooler.model.AgentActivity,
        /**
         * Whether that attention is a turn that DIED. Carried beside
         * [attention] because [com.farcooler.model.AgentActivity] has no word
         * for it — the daemon sends a failed turn as `done` — and without it
         * this ring was green around an agent that had stopped working.
         */
        val turnDidFail: Boolean,
    )

    val chips = entries.flatMap { entry ->
        val numbering = entry.workspace.ordinals()
        entry.workspace.terminals.map { terminal ->
            Chip(
                ref = TerminalRef(entry.host.id, entry.workspace.id, terminal.id),
                label = terminal.displayName(numbering[terminal.id]),
                runner = entry.host.displayLabel,
                kind = StateKind.parse(terminal.state),
                wantsAttention = terminal.agent.wantsAttention,
                attention = terminal.agent,
                turnDidFail = terminal.turnDidFail,
            )
        }
    }
    if (chips.isEmpty()) return

    val state = rememberLazyListState()
    val index = chips.indexOfFirst { it.ref.terminalId == current.terminalId }

    // Scrolled to the open terminal when it changes, not on every fleet
    // refresh: a poll landing mid-scroll must not fight the user's own gesture.
    LaunchedEffect(current.terminalId) {
        if (index >= 0) state.animateScrollToItem(index)
    }

    LazyRow(
        state = state,
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 6.dp)
            // ONE surface holding them, not one per chip. A chip apiece on its
            // own background reads as a browser tab strip pasted under a
            // terminal; a single bar is one sibling of the screen above it.
            .clip(RoundedCornerShape(18.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHigh),
        contentPadding = PaddingValues(horizontal = 6.dp, vertical = 5.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        items(chips.size, key = { chips[it].ref.hostId + "/" + chips[it].ref.terminalId }) { i ->
            val chip = chips[i]
            val isCurrent = chip.ref.terminalId == current.terminalId
            Row(
                Modifier
                    .clip(CircleShape)
                    .background(
                        if (isCurrent) MaterialTheme.colorScheme.secondaryContainer
                        else Color.Transparent
                    )
                    .then(
                        if (chip.wantsAttention) {
                            Modifier.border(
                                1.5.dp,
                                attentionColor(chip.attention, chip.turnDidFail),
                                CircleShape,
                            )
                        } else {
                            Modifier
                        }
                    )
                    .clickable { onSelect(chip.ref) }
                    .padding(horizontal = 10.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // Same dot, same size, same rules as the fleet list —
                // [ProcessDot] is shared rather than redrawn here so a terminal
                // cannot read one way in one screen and another in the other.
                // This drew its own 6dp circle while the list drew 8dp, which is
                // exactly the drift the Mac's `StatusGlyph` had to be pulled
                // back from.
                //
                // A running pane draws nothing at all now. The chip does not
                // collapse when it does: the box is claimed whether or not
                // anything occupies it, so a pane starting or dying does not
                // shove every chip after it sideways.
                ProcessDot(chip.kind)
                Spacer(Modifier.width(6.dp))
                Text(
                    // Capped, because a chip carries the CONVERSATION's name
                    // rather than "claude 2", and an agent will happily call
                    // one "Complete D17 authorization decision for Far Cooler"
                    // — which filled the strip with a single tab and pushed
                    // every other pane off the end of it.
                    if (showRunner) "${chip.label} · ${chip.runner}" else chip.label,
                    style = MaterialTheme.typography.labelLarge,
                    fontWeight = if (chip.wantsAttention) FontWeight.SemiBold else FontWeight.Normal,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    color =
                        if (isCurrent) MaterialTheme.colorScheme.onSecondaryContainer
                        else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.widthIn(max = 160.dp),
                )
            }
        }
    }
}
