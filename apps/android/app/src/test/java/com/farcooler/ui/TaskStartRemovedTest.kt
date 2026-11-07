package com.farcooler.ui

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Nothing in the app starts a task from typed words (ov-951). A person tells
 * the orchestrator, and the orchestrator starts the work. New worktree stays:
 * it makes a worktree the person names, not a task.
 */
class TaskStartRemovedTest {
    private val markers = listOf(
        "QuickTaskSheet", "showQuickTask", "describing = true", "fun Connection.writeRaw", "hexEncode(",
    )

    private fun mainSources(): List<File> {
        // Gradle runs unit tests from the module directory.
        val root = listOf(File("src/main/java"), File("app/src/main/java")).first { it.isDirectory }
        return root.walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()
    }

    private fun startsFound(texts: List<Pair<String, String>>): List<String> =
        texts.flatMap { (name, text) -> markers.filter { text.contains(it) }.map { "$it in $name" } }

    @Test
    fun noSourceOffersAWayToStartATaskFromADescription() {
        val sources = mainSources()
        assertTrue("found ${sources.size} sources, so this proves nothing", sources.size > 100)
        val texts = sources.map { it.name to it.readText() }
        assertTrue("New worktree moved", texts.any { it.second.contains("fun NewWorktreeSheet(") })
        assertEquals(emptyList<String>(), startsFound(texts))
    }

    @Test
    fun aPlantedStartIsFound() {
        val texts = listOf(
            "A.kt" to "if (describing) QuickTaskSheet(model)",
            "B.kt" to "onClick = { describing = true }",
            "C.kt" to "NewWorktreeSheet(model)",
        )
        assertEquals(listOf("QuickTaskSheet in A.kt", "describing = true in B.kt"), startsFound(texts))
    }
}
