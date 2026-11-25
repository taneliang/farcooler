package com.farcooler.ui

import android.content.ClipData
import androidx.compose.foundation.draganddrop.dragAndDropSource
import androidx.compose.foundation.draganddrop.dragAndDropTarget
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DragHandle
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draganddrop.DragAndDropEvent
import androidx.compose.ui.draganddrop.DragAndDropTarget
import androidx.compose.ui.draganddrop.DragAndDropTransferData
import androidx.compose.ui.draganddrop.toAndroidDragEvent
import androidx.compose.ui.unit.dp
import com.farcooler.model.TaskRow

internal data class BoardDrag(val board: String, val task: String)

internal class BoardReorder(
    board: String,
    private val section: List<TaskRow>,
    row: TaskRow,
    private val submit: (task: String, after: Boolean) -> Unit,
) {
    val drag = BoardDrag(board, row.id)

    fun accepts(source: BoardDrag): Boolean =
        source.board == drag.board && source.task != drag.task && section.any { it.id == source.task }

    fun drop(source: BoardDrag): Boolean {
        if (!accepts(source)) return false
        val from = section.indexOfFirst { it.id == source.task }
        val to = section.indexOfFirst { it.id == drag.task }
        submit(source.task, from < to)
        return true
    }
}

@Composable
internal fun boardDropTarget(reorder: BoardReorder?): Modifier {
    if (reorder == null) return Modifier
    val current = rememberUpdatedState(reorder)
    val target = remember {
        object : DragAndDropTarget {
            override fun onDrop(event: DragAndDropEvent): Boolean {
                val source = event.toAndroidDragEvent().localState as? BoardDrag ?: return false
                return current.value.drop(source)
            }
        }
    }
    return Modifier.dragAndDropTarget(
        shouldStartDragAndDrop = { event ->
            val source = event.toAndroidDragEvent().localState as? BoardDrag
            source != null && reorder.accepts(source)
        },
        target = target,
    )
}

@Composable
internal fun BoardDragHandle(drag: BoardDrag) {
    Box(
        Modifier.size(40.dp).dragAndDropSource(
            transferData = { DragAndDropTransferData(ClipData.newPlainText("Ticket", drag.task), localState = drag) },
        ),
        contentAlignment = Alignment.Center,
    ) {
        Icon(Icons.Outlined.DragHandle, contentDescription = "Drag to reorder")
    }
}
