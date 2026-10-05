package com.farcooler.capture

import com.farcooler.model.Plan
import com.farcooler.model.TaskBoard
import com.farcooler.model.TaskKeyCards
import com.farcooler.ui.TaskCardRow
import com.farcooler.ui.TaskKeyCardDialog
import androidx.compose.foundation.layout.Column
import com.farcooler.model.TaskAgentPresence
import java.io.File
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * A long press on a task key (ov-299): the board's own row, and the card it
 * opens, from the CLI's own board and plan (`test/fixtures/task-key-cards`,
 * what `farcooler task list --json` and `plan --json` printed for the seeded
 * Billing board). Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class TaskKeyCardCaptureTest {
    private val board = TaskBoard.decode(repositoryFile("test/fixtures/task-key-cards/tasks.json"))
    private val plan = Plan.decode(repositoryFile("test/fixtures/task-key-cards/plan.json"))
    private val cards = TaskKeyCards.of("r1", mapOf("w" to board), mapOf("w" to plan))

    @Test fun heldKey() = Capture.bothWithDialog("task-key-card") {
        Column {
            for (row in board.rows) {
                TaskCardRow(
                    row, agents = emptyList(), orchestrator = null, speaks = true,
                    presence = TaskAgentPresence.Unsaid, onOpen = {}, onJump = {}, card = cards.card(row.key),
                )
            }
        }
        TaskKeyCardDialog(cards.card("bil-2")!!, onOpen = {}, onCopy = {}, onDismiss = {})
    }

    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}")
    }
}
