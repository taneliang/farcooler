package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AccountTree
import androidx.compose.material.icons.outlined.CallMerge
import androidx.compose.material.icons.outlined.ChatBubbleOutline
import androidx.compose.material.icons.outlined.PanTool
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.dp
import com.farcooler.model.EmptyIcon
import com.farcooler.model.PhoneEmptyCopy

/** The Material icon for a row's [EmptyIcon]. */
fun EmptyIcon.vector(): ImageVector = when (this) {
    EmptyIcon.BUBBLE -> Icons.Outlined.ChatBubbleOutline
    EmptyIcon.HAND -> Icons.Outlined.PanTool
    EmptyIcon.EYE -> Icons.Outlined.Visibility
    EmptyIcon.ADD_PERSON -> Icons.Outlined.PersonAdd
    EmptyIcon.BRANCH -> Icons.Outlined.AccountTree
    EmptyIcon.MERGE -> Icons.Outlined.CallMerge
}

/**
 * An empty state's icon rows (ov-245): the iPhone's `PhoneEmptyRows`. Each icon
 * sits in a fixed-width column so the words line up, the rows as one
 * leading-aligned block as wide as its widest row, centered under the lede
 * and capped so nothing wraps to a one-word line (ov-266). The lede is the caller's `EmptyState` detail, so it
 * stays where every other empty state's sentence is, with a little more room
 * under it than the rows have between them, so it doesn't read as another row.
 */
@Composable
fun PhoneEmptyRows(copy: PhoneEmptyCopy, modifier: Modifier = Modifier) {
    Column(
        modifier.padding(top = 8.dp).widthIn(max = 320.dp).testTag("empty-rows"),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        copy.rows.forEach { row ->
            Row(
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.testTag("empty-row"),
            ) {
                Icon(
                    row.icon.vector(),
                    contentDescription = null,
                    modifier = Modifier.size(22.dp).clearAndSetSemantics {},
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    row.text,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
