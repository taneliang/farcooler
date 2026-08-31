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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Difference
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.InboxRow
import com.farcooler.model.StateKind
import com.farcooler.model.Workspace

/**
 * One workspace's tabs: its agents, and its diff.
 *
 * The Mac always has its sidebar up, so switching panes there is a click away
 * regardless of which is open. A phone's workspace screen is full-bleed, and
 * "open the drawer, find the row, tap it" is the wrong cost for something as
 * routine as glancing at a second agent.
 *
 * ## Why this is scoped to one workspace now
 *
 * It used to be flat across the whole fleet, and the argument for that was
 * written down right here: the 3am case is "is the OTHER agent still blocked",
 * which is as likely to be on a different runner as in the same worktree. That
 * is still true, and it stopped being the case this strip has to serve.
 *
 * The job the owner described is reviewing what an agent did — reading what it
 * said, judging it, looking at the change, replying — and every one of those is
 * inside ONE worktree. A flat strip cannot hold that worktree's diff, because a
 * diff belongs to a workspace and a flat strip has no workspace; and if it did
 * hold one it would be a lone unlabeled chip in a row of ten unrelated ones.
 * Scoped, the strip is the toggle: the agents that did the work, and the work
 * they did, side by side.
 *
 * The cross-worktree jump is not lost and did not even move far. Back is
 * [NeedsYouScreen], which answers "is the other agent still blocked" with a
 * ranked sentence rather than with a chip's dot, and it now spans every runner
 * — which the flat strip did too, and which is the thing this app must not give
 * up. The drawer is still an edge swipe away on this very screen and still
 * lists the whole fleet. Two ways out, both of them better at the job than a
 * row of chips for panes in other worktrees.
 *
 * ## The position is not up for discussion
 *
 * **Bottom, in thumb reach.** iOS moved its strip to a floating overlay under
 * the navigation bar; `cb13d31` and the parity inventory both say Android's
 * stays where it is, and the reason is stated on the screen that owns it. This
 * change is to the strip's SCOPE, not its position.
 *
 * A host-side `changes` pane gets no chip of its own. The Changes chip already
 * is that pane's review, and two chips onto one diff is a choice with no
 * difference behind it — see [Pane].
 */
@Composable
fun TerminalTabStrip(
    /**
     * The workspace whose tabs these are.
     *
     * Nullable only for the moment between this screen appearing and the
     * fleet's next answer. The Changes chip stands on its own until then,
     * because the diff is asked for by workspace id and needs nothing from the
     * fleet to be worth a chip.
     */
    workspace: Workspace?,
    /** What this worktree has changed, for the Changes chip's counts. */
    counts: InboxRow?,
    current: Pane,
    onSelect: (Pane) -> Unit,
    modifier: Modifier = Modifier,
) {
    data class Chip(
        val pane: Pane,
        val label: String,
        val kind: StateKind,
        val wantsAttention: Boolean,
        /**
         * The whole terminal, for [agentTint].
         *
         * The activity and the `turnFailed` flag travelled here as two fields
         * for a while, so that the ring could be orange, green or red without
         * the chip holding a model object. That was the wrong economy: the rule
         * for which of the three it is lives in ONE function now — the same one
         * the fleet row and the Mac's glyph use — and a chip that flattened its
         * terminal into two booleans before asking would be a third place that
         * could get the flattening wrong.
         */
        val terminal: com.farcooler.model.Terminal,
    )

    val numbering = workspace?.ordinals() ?: emptyMap()
    val chips = workspace?.terminals.orEmpty()
        .filterNot { it.isChangesPane }
        .map { terminal ->
            Chip(
                pane = Pane.Terminal(terminal.id),
                label = terminal.displayName(numbering[terminal.id]),
                kind = StateKind.parse(terminal.state),
                wantsAttention = terminal.agent.wantsAttention,
                terminal = terminal,
            )
        }

    val state = rememberLazyListState()
    // +1 for the Changes chip, which leads and is not in [chips].
    val index = chips.indexOfFirst { it.pane == current }.let { if (it < 0) 0 else it + 1 }

    // Scrolled to the open tab when it changes, not on every fleet refresh: a
    // poll landing mid-scroll must not fight the user's own gesture.
    LaunchedEffect(current) { state.animateScrollToItem(index) }

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
        // Changes leads, so the one chip that is always there is always in the
        // same place — the diff is what a thumb can find without reading, and
        // the agents shuffle around it as panes come and go.
        item(key = Pane.CHANGES_ID) {
            ChangesChip(
                counts = counts,
                isCurrent = current is Pane.Changes,
                onTap = { onSelect(Pane.Changes) },
            )
        }

        items(chips.size, key = { chips[it].pane.id }) { i ->
            val chip = chips[i]
            val isCurrent = chip.pane == current
            Row(
                Modifier
                    .clip(CircleShape)
                    .background(
                        if (isCurrent) MaterialTheme.colorScheme.secondaryContainer
                        else Color.Transparent
                    )
                    // Amber for blocked, the review ink for a finished turn,
                    // red for one that died — from [agentTint], which is the one
                    // place that rule lives and which the fleet row and the
                    // Mac's glyph read too. Blocked's amber is now the palette's
                    // rather than Material orange 500, and amber and review are
                    // both §01's: red is the only hue left here that is this
                    // app's own ink, for the one outcome the glance vocabulary
                    // still has no mark for.
                    //
                    // **A finished turn is visibly a different colour on this
                    // chip than it was.** It was green until `done` joined the
                    // review tier; the tier is what a person is being told, and
                    // green was saying "it worked" where the useful fact is "you
                    // have not looked at it". The ring here and the mark the
                    // fleet row draws for the same terminal are the same ink,
                    // because [agentInk] decides it once for both.
                    //
                    // **This chip is exactly why red survived.** The argument for
                    // folding a failed turn into amber was that the word is
                    // printed beside the mark — true of a fleet row, false here.
                    // A chip carries a conversation's NAME and no state text at
                    // all, so on this surface the hue is the whole distinction
                    // between a turn that worked and a turn that died.
                    //
                    // **A ring and not a mark, and only for now.** §03's
                    // vocabulary would put an `AgentMarkView` at
                    // `GlanceMarkSize.RIBBON` in this chip's leading slot — the
                    // slot [ProcessDot] currently holds — and that is a change
                    // to what the strip IS rather than to what it is coloured.
                    // The strip is the surface iOS replaced wholesale with the
                    // shell's bar; restructuring it here would be work thrown
                    // away twice. So this pass takes the hue only, which is the
                    // half that is wrong today independently of what the strip
                    // becomes.
                    .then(
                        when (val tint = agentTint(chip.terminal)) {
                            null -> Modifier
                            else -> Modifier.border(1.5.dp, tint, CircleShape)
                        }
                    )
                    .clickable { onSelect(chip.pane) }
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
                    //
                    // The runner is gone from the label. It was here while this
                    // strip spanned the fleet and two chips could be two
                    // machines; a scoped strip is one workspace on one runner,
                    // and the title bar above already names it when more than
                    // one is connected.
                    chip.label,
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

/**
 * The worktree's own tab: what the branch changed.
 *
 * Always there, including on a workspace with no panes at all, because the diff
 * is asked for by workspace id — see [Pane]. That is also why it needs no
 * terminal, no dot and no state: nothing about it can be starting, exited or
 * lost.
 *
 * **Deliberately not amber.** Orange means an agent is waiting on an answer, on
 * this strip and on the fleet list and the front door, and a glance at any of
 * them has to answer "does this need me" without reading a word. A diff waiting
 * to be read is worth showing; it is not worth the color that means somebody is
 * stuck. `NeedsYouScreen`'s review row makes the same argument about the same
 * fact.
 *
 * So the counts carry it instead, in the green and red they have everywhere
 * else — which says how big the change is as well as that there is one, in the
 * space a badge would have taken.
 */
@Composable
private fun ChangesChip(counts: InboxRow?, isCurrent: Boolean, onTap: () -> Unit) {
    Row(
        Modifier
            .clip(CircleShape)
            .background(
                if (isCurrent) MaterialTheme.colorScheme.secondaryContainer
                else Color.Transparent
            )
            .clickable(onClick = onTap)
            .padding(horizontal = 10.dp, vertical = 6.dp)
            // There is no hover on a phone, so this is the only place the chip
            // can say WHAT it counts: everything the worktree has changed, not
            // the branch total the workspace list shows under Branch. The fleet
            // list's workspace header and the front door both say the same
            // clause in the same place and for the same reason.
            .semantics {
                contentDescription = changesDescription(counts)
            },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            // Material's own mark for a comparison of two things, which is what
            // a diff is. The Apple apps print `plusminus`; Android has no
            // guarantee of that glyph in the system font, and the counts three
            // dp to the right already carry the two signs.
            Icons.Filled.Difference,
            contentDescription = null,
            modifier = Modifier.size(14.dp),
            tint =
                if (isCurrent) MaterialTheme.colorScheme.onSecondaryContainer
                else MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.width(6.dp))
        Text(
            "Changes",
            style = MaterialTheme.typography.labelLarge,
            maxLines = 1,
            color =
                if (isCurrent) MaterialTheme.colorScheme.onSecondaryContainer
                else MaterialTheme.colorScheme.onSurfaceVariant,
        )
        // Absent entirely on a clean worktree. `+0 -0` on every branch with
        // nothing on it is noise in the shape of information — the fleet list's
        // workspace header leaves it out for the same reason.
        if (counts != null && counts.hasDiff) {
            Spacer(Modifier.width(6.dp))
            DiffCounts(counts)
        }
    }
}

/**
 * What the Changes chip says to a screen reader — and the front door's review
 * row with it.
 *
 * Pure and out here rather than built inline, so a test can read the one clause
 * that only exists in this string: the counts are everything the worktree has
 * changed, committed or not, which is not the branch total the workspace list
 * shows under Branch.
 *
 * [lead] is the only thing the two callers differ on, because the two are the
 * same target on two screens: `NeedsYouScreen`'s row says "Review changes" and
 * this chip says "Changes", and everything after the comma has to match or the
 * app describes one worktree two ways on either side of one tap. That was
 * unfalsifiable while the row led somewhere else; it is checkable now.
 */
internal fun changesDescription(counts: InboxRow?, lead: String = "Changes"): String {
    if (counts == null || !counts.hasDiff) return lead
    return "$lead, ${counts.insertions} added, ${counts.deletions} removed, " +
        "including work that isn’t committed yet"
}
