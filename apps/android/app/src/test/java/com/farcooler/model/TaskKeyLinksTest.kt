package com.farcooler.model

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.LinkAnnotation
import com.farcooler.ui.Route
import com.farcooler.ui.TaskKeyLinker
import com.farcooler.ui.actions
import com.farcooler.ui.inlineAnnotated
import com.farcooler.ui.route
import com.farcooler.ui.taskKeyLinker
import com.farcooler.ui.targets
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Task keys in text as links (ov-196): AgentKit's `TaskKeyLinksTests`, against the same fixture. */
class TaskKeyLinksTest {
    /** The boards runner "r1" has read: ov-190 and ov-7 on Main, lo-3 on another workspace. */
    private fun index(): TaskKeyIndex {
        fun board(vararg rows: Pair<String, String>) = TaskBoard(
            listOf(TaskBoardColumn(TaskStatus.TODO, rows.map { (id, key) -> TaskRow(id, key, key, TaskStatus.TODO, 0L) })),
        )
        return TaskKeyIndex.of(
            "r1",
            listOf(WorkspaceSummary("w-main", taskPrefix = "ov"), WorkspaceSummary("w-lo", taskPrefix = "lo")),
            mapOf("w-main" to board("t190" to "ov-190", "t7" to "ov-7"), "w-lo" to board("t3" to "lo-3")),
        )
    }

    @Test
    fun `every case in the shared fixture finds exactly its keys`() {
        val root = Json.parseToJsonElement(repositoryFile("test/fixtures/task-key-links.json")).jsonObject
        val prefixes = root.getValue("prefixes").jsonArray.map { it.jsonPrimitive.content }.toSet()
        val known = root.getValue("known").jsonArray.map { it.jsonPrimitive.content }.toSet()
        val cases = root.getValue("cases").jsonArray
        assertTrue(cases.size >= 20)
        for (case in cases) {
            val text = case.jsonObject.getValue("text").jsonPrimitive.content
            val expected = case.jsonObject.getValue("links").jsonArray.map {
                TaskKeyLinks.Match(it.jsonObject.getValue("start").jsonPrimitive.int, it.jsonObject.getValue("key").jsonPrimitive.content)
            }
            assertEquals(text, expected, TaskKeyLinks.matches(text, prefixes, known))
        }
    }

    @Test
    fun `a runner's index is its workspaces' prefixes and its boards' keys`() {
        val index = index()
        assertEquals(setOf("ov", "lo"), index.prefixes)
        assertEquals(TaskKeyTarget("r1", "w-main", "t190", "ov-190"), index.targets["ov-190"])
        assertEquals(listOf("ov-190", "lo-3"), TaskKeyLinks.matches("ov-190, ov-191 and lo-3", index).map { it.key })
        assertTrue(TaskKeyLinks.matches("ov-190", TaskKeyIndex.EMPTY).isEmpty())
    }

    @Test
    fun `a task link names its runner and key, and nothing else parses as one`() {
        val url = TaskKeyLinks.url("e@host:22/x", "ov-190")
        assertEquals("e@host:22/x" to "ov-190", TaskKeyLinks.parse(url))
        assertEquals("farcooler://task/r1/ov-190", TaskKeyLinks.url("r1", "ov-190"))
        for (other in listOf(
            "farcooler://task/r1", "farcooler://task/r1/ov-1/x", "farcooler://terminal/r1/ov-1",
            "farcooler://x", "https://task/r1/ov-1", "farcooler-canary://task/r1/ov-1",
        )) {
            assertNull(other, TaskKeyLinks.parse(other))
        }
    }

    /** A key in a line becomes a clickable link; in code or a link's words it doesn't. */
    @Test
    fun `a key in text links to its task, and a click opens it on its own runner`() {
        val opened = mutableListOf<TaskKeyTarget>()
        val linker = TaskKeyLinker(index()) { opened.add(it) }
        fun links(markdown: String) = inlineAnnotated(Markdown.inline(markdown), linker, Color.Gray, Color.Blue).let { text ->
            text.getLinkAnnotations(0, text.length).map { text.substring(it.start, it.end) to it.item }
        }
        val found = links("Done in **ov-190**, not utf-8 or ov-191.")
        assertEquals(listOf("ov-190"), found.map { it.first })
        assertTrue(links("`ov-190` is quoted").isEmpty())
        assertTrue(links("[see ov-190](https://x.y/ov-190)").isEmpty())
        assertTrue(inlineAnnotated(Markdown.inline("ov-190"), TaskKeyLinker.NONE, Color.Gray, Color.Blue)
            .getLinkAnnotations(0, 6).isEmpty())

        val link = found.single().second as LinkAnnotation.Clickable
        assertEquals("farcooler://task/r1/ov-190", link.tag)
        link.linkInteractionListener!!.onClick(link)
        assertEquals(listOf(TaskKeyTarget("r1", "w-main", "t190", "ov-190")), opened)

        assertTrue("another runner's: taken, not opened", linker.follow("farcooler://task/r2/ov-190"))
        assertTrue("unknown: taken, not opened", linker.follow("farcooler://task/r1/ov-999"))
        assertFalse(linker.follow("https://a.b"))
        assertEquals(1, opened.size)
    }

    /** Opened as a board row or History opens a task: its own task screen, on its runner. */
    @Test
    fun `a task link opens through the task screen's route`() {
        val target = index().targets.getValue("lo-3")
        assertEquals(Route.BoardTask("r1", "w-lo", "t3"), target.route())
    }

    /**
     * The screens' own builder, as `rememberTaskKeyLinker` calls it: following
     * a link hands the navigator its task's screen, each id where it goes.
     */
    @Test
    fun `the app's linker navigates to the linked task's screen`() {
        val navigated = mutableListOf<Route>()
        val boards = mapOf(
            "w-lo" to TaskBoard(listOf(TaskBoardColumn(TaskStatus.TODO, listOf(TaskRow("t3", "lo-3", "T", TaskStatus.TODO, 0L))))),
        )
        val linker = taskKeyLinker("r1", listOf(WorkspaceSummary("w-lo", taskPrefix = "lo")), boards) { navigated.add(it) }
        assertTrue(linker.follow(TaskKeyLinks.url("r1", "lo-3")))
        assertEquals(listOf<Route>(Route.BoardTask("r1", "w-lo", "t3")), navigated)
    }

    /** A row whose description replaces its text still offers each linked task once, as an action. */
    @Test
    fun `a linked line's tasks become one action apiece`() {
        val opened = mutableListOf<TaskKeyTarget>()
        val linker = TaskKeyLinker(index()) { opened.add(it) }
        val text = inlineAnnotated(
            Markdown.inline("lo-3 before ov-190, `ov-7`, [ov-7](https://x.dev) and lo-3 again"), linker, Color.Gray, Color.Blue,
        )
        assertEquals(listOf("lo-3", "ov-190"), linker.targets(text).map { it.key })
        val actions = linker.actions(text)
        assertEquals(listOf("Open lo-3", "Open ov-190"), actions.map { it.label })
        assertTrue(actions[1].action())
        assertEquals(listOf("ov-190"), opened.map { it.key })
    }

    /** A link's target as a `Destination` (ov-182), in this device's ids: AgentKit's `TaskKeyTarget.destination`. */
    @Test
    fun `a task link's target as a destination`() {
        assertEquals(
            Destination(runner = Destination.Runner(host = "r1"), place = Destination.Place.Task("w-lo", Destination.TaskRef(id = "t3", key = "lo-3"))),
            index().targets.getValue("lo-3").destination,
        )
    }

    /** A file in this checkout, found by walking up from wherever Gradle runs. */
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
