package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.outlined.Train
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.GlancePalette
import com.farcooler.model.PageLinks
import com.farcooler.model.PlanCiRead
import com.farcooler.model.PlanTrain
import com.farcooler.model.TrainWords

/**
 * A train heading its lanes in Now (ov-309), as on the Mac and the iPhone: its
 * name, where it stands and its CI as the runner last read it. Red, or a
 * failed run, is the one thing drawn in amber, with its word. A tap opens the
 * CI run in the browser, the one place it goes.
 */
@Composable
fun PlanTrainRow(train: PlanTrain, ci: PlanCiRead?, now: Long = 0) {
    val words = TrainWords.train(train, ci, now)
    val attention = TrainWords.needsAttention(train, ci)
    val amber = glanceColor(GlancePalette.amber)
    val run = ci?.url?.takeIf { PageLinks.https(it) != null }
    val uri = LocalUriHandler.current
    val tint = if (attention) amber else MaterialTheme.colorScheme.onSurfaceVariant
    ListItem(
        leadingContent = {
            Box(Modifier.width(24.dp), contentAlignment = Alignment.CenterStart) {
                Icon(Icons.Outlined.Train, contentDescription = null, modifier = Modifier.size(20.dp), tint = tint)
            }
        },
        headlineContent = { Text(train.heading, style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.SemiBold), maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            Column {
                train.carries?.let {
                    Text(it, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("plan-train-${train.name}-carries"))
                }
                train.slug?.let {
                    Text(it, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.labelSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Text(
                    words,
                    color = if (attention) amber else MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.testTag("plan-train-${train.name}-words"),
                )
            }
        },
        trailingContent = {
            if (run != null) Icon(Icons.AutoMirrored.Outlined.OpenInNew, contentDescription = null, tint = MaterialTheme.colorScheme.outline)
        },
        modifier = (if (run != null) Modifier.clickable(role = Role.Button) { uri.openUri(run) } else Modifier)
            .testTag("plan-train-${train.name}")
            .semantics(mergeDescendants = true) { contentDescription = listOfNotNull(train.heading, train.carries, words).joinToString(", ") },
    )
}
