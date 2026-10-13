package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.GlancePalette
import com.farcooler.model.PlanGlanceSentence

// The Plan's glance band (ov-503, design 2), at the top of the plan: one line for Now and one for Next, and what
// needs the owner as the one amber mark. The words are `PlanGlanceSentence`'s, which the strip, the watch and the Mac
// read too.
//
//   Now   Train 86 waiting for CI · 2 building · 3 in review
//   Next  Reps: each person's asks reach the orchestrator, then Onboarding…
//   ● 2 need you

/** "1 needs you", "2 need you". */
fun needsYouWords(count: Int): String = "$count ${if (count == 1) "needs" else "need"} you"

@Composable
fun PlanGlanceBand(
    band: PlanGlanceSentence,
    needsYou: Int,
    modifier: Modifier = Modifier,
    onNeedsYou: (() -> Unit)? = null,
    /** Whether the pill is drawn; off where a Needs you section follows directly and says it. */
    pill: Boolean = true,
) {
    val amber = glanceColor(GlancePalette.amber)
    val said = if (needsYou > 0) "${band.text} ${needsYouWords(needsYou)}." else band.text
    Column(
        modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag("plan-band")
            .semantics(mergeDescendants = true) { contentDescription = said },
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        line("Now", band.nowLine, quiet = band.now == null, tag = "plan-band-now")
        line("Next", band.nextLine, quiet = band.next == null, tag = "plan-band-next")
        if (needsYou > 0 && pill) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = (if (onNeedsYou != null) Modifier.clickable(role = Role.Button, onClick = onNeedsYou) else Modifier)
                    .heightIn(min = if (onNeedsYou != null) 48.dp else 0.dp).testTag("plan-band-needs-you"),
            ) {
                // The dot carries the color; the words stay the text's own (review 17).
                Box(Modifier.size(8.dp).clip(CircleShape).background(amber))
                Spacer(Modifier.width(8.dp))
                Text(needsYouWords(needsYou), style = MaterialTheme.typography.labelLarge.copy(fontWeight = FontWeight.SemiBold))
            }
        }
    }
}

@Composable
private fun line(label: String, words: String, quiet: Boolean, tag: String) {
    Row(verticalAlignment = Alignment.Top, modifier = Modifier.testTag(tag)) {
        Text(
            label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.width(40.dp).padding(top = 2.dp),
        )
        Text(
            words,
            style = MaterialTheme.typography.bodyMedium.copy(fontWeight = if (quiet) FontWeight.Normal else FontWeight.Medium),
            color = if (quiet) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
            maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
        )
    }
}
