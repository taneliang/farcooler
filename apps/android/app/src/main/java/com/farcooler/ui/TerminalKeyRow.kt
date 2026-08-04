package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardTab
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.material.icons.filled.East
import androidx.compose.material.icons.filled.West
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.KeyboardHide
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import com.farcooler.core.Vt

/**
 * The row of keys a terminal needs and a phone's keyboard does not have.
 *
 * Nine keys share the width rather than each claiming their own. Sized from
 * their own content they overflowed a phone on iOS — a bordered button pads
 * whatever you hand it — and the row did not clip on its own: it widened the
 * stack it was in, so the terminal ABOVE it lost characters off both edges.
 * `weight(1f)` makes overflow impossible to express.
 */
@Composable
fun TerminalKeyRow(
    ctrlArmed: Boolean,
    altArmed: Boolean,
    onToggleCtrl: () -> Unit,
    onToggleAlt: () -> Unit,
    onKey: (Int) -> Unit,
    onDismiss: () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 6.dp, vertical = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Key(onClick = { onKey(Vt.KEY_ESCAPE) }) { Legend("esc") }
        Key(onClick = { onKey(Vt.KEY_TAB) }) {
            Icon(Icons.AutoMirrored.Filled.KeyboardTab, contentDescription = "Tab", Modifier.size(18.dp))
        }
        Key(filled = ctrlArmed, onClick = onToggleCtrl) { Legend("ctrl") }
        Key(filled = altArmed, onClick = onToggleAlt) { Legend("alt") }

        // Held, each arrow becomes the jump it is the small version of. A phone
        // has no room for eight more keys and no modifier to hide them behind,
        // and holding a direction to go further in it is the gesture people
        // already have for exactly this.
        Arrow(Icons.Filled.West, "Left", Vt.KEY_LEFT, Vt.KEY_HOME, onKey)
        Arrow(Icons.Filled.ArrowDownward, "Down", Vt.KEY_DOWN, Vt.KEY_PAGE_DOWN, onKey)
        Arrow(Icons.Filled.ArrowUpward, "Up", Vt.KEY_UP, Vt.KEY_PAGE_UP, onKey)
        Arrow(Icons.Filled.East, "Right", Vt.KEY_RIGHT, Vt.KEY_END, onKey)

        // Putting the keyboard away, which this row is otherwise the only thing
        // standing in the way of: it lives above the keyboard, so it goes when
        // the keyboard does, and without a way to dismiss from here there is
        // nowhere else to ask from.
        Key(onClick = onDismiss) {
            Icon(Icons.Filled.KeyboardHide, contentDescription = "Hide the keyboard", Modifier.size(18.dp))
        }
    }
}

@Composable
private fun RowScope.Arrow(
    icon: ImageVector,
    label: String,
    tap: Int,
    hold: Int,
    onKey: (Int) -> Unit,
) {
    Key(onClick = { onKey(tap) }, onLongClick = { onKey(hold) }) {
        Icon(icon, contentDescription = label, Modifier.size(18.dp))
    }
}

@Composable
private fun RowScope.Key(
    filled: Boolean = false,
    onClick: () -> Unit,
    onLongClick: (() -> Unit)? = null,
    content: @Composable () -> Unit,
) {
    Box(
        Modifier
            .weight(1f)
            .height(42.dp)
            .clip(RoundedCornerShape(7.dp))
            .background(
                if (filled) MaterialTheme.colorScheme.primary
                else MaterialTheme.colorScheme.surfaceVariant
            )
            // A long press that fires once rather than repeating: these send a
            // jump, and a jump that repeated while a thumb rested on it would
            // scroll somewhere nobody asked to be.
            .combinedClickable(onClick = onClick, onLongClick = onLongClick),
        contentAlignment = Alignment.Center,
    ) {
        androidx.compose.runtime.CompositionLocalProvider(
            androidx.compose.material3.LocalContentColor provides
                if (filled) MaterialTheme.colorScheme.onPrimary
                else MaterialTheme.colorScheme.onSurfaceVariant
        ) {
            content()
        }
    }
}

@Composable
private fun Legend(text: String) {
    Text(text, style = MaterialTheme.typography.labelMedium)
}
