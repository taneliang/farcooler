package com.farcooler.model

import java.io.File
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The orchestrator owns the task list (ov-184): this app answers an agent's
 * question and writes nothing else to a task.
 */
class TaskAnswerTest {
    /**
     * An answer goes out as `task.note` with `kind: answer`, the one kind the
     * client core still takes from a phone; anything else it refuses before
     * the runner sees it, and the waiting agent would never wake.
     */
    @Test
    fun `an answer is a task note of kind answer`() {
        assertEquals("task.note", TaskAnswer.METHOD)
        val sent = TaskAnswer.request("0192-task", "Postgres")
        assertEquals(setOf("task", "kind", "body"), sent.keys)
        assertEquals("0192-task", sent.getValue("task").jsonPrimitive.content)
        assertEquals("answer", sent.getValue("kind").jsonPrimitive.content)
        assertEquals("Postgres", sent.getValue("body").jsonPrimitive.content)
    }

    /**
     * No source in this app names a task write the orchestrator owns, so no
     * screen, receiver or model can reach one through the client core. The
     * core has no arm for them either (`no_phone_can_write_a_task` in
     * `crates/client`); this keeps the app from growing a call that would
     * only fail at run time.
     */
    @Test
    fun `no app source names a task write the orchestrator owns`() {
        val main = mainSources()
        val sources = main.walkTopDown().filter { it.isFile && it.extension == "kt" }.toList()
        assertTrue("found ${sources.size} sources under $main, so this proves nothing", sources.size > 50)
        // Proves the scan reads string literals: the answer path is found.
        assertTrue(sources.any { "\"task.note\"" in it.readText() })

        val writes = listOf("task.create", "task.update", "task.set_status", "task.move", "task.block")
        val named = sources.flatMap { file ->
            val text = file.readText()
            writes.filter { "\"$it\"" in text }.map { "$it in ${file.name}" }
        }
        assertEquals("a task write the orchestrator owns", emptyList<String>(), named)
    }

    /** This app's `src/main`, found by walking up from wherever Gradle runs. */
    private fun mainSources(): File {
        val relative = "apps/android/app/src/main/java"
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isDirectory) return candidate
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}")
    }
}
