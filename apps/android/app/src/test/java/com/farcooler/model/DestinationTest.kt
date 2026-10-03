package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.double
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Destinations, held to `test/fixtures/destinations.json`, which AgentKit's
 * `DestinationTests` replays too: the encoding byte for byte, every
 * notification spelling already shipped, the links, and the resolver.
 */
class DestinationTest {
    private val root = Json.parseToJsonElement(repositoryFile("test/fixtures/destinations.json")).jsonObject

    private fun cases(section: String) = root[section]!!.jsonArray.map { it.jsonObject }
    private fun JsonObject.text(key: String): String? = (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content
    private fun JsonObject.name() = text("case")!!

    @Test
    fun `encodings round trip byte for byte`() {
        val cases = cases("encodings")
        assertTrue(cases.size >= 10)
        for (case in cases) {
            val json = case.text("json")!!
            assertEquals(case.name(), json, Destination.decode(json)?.encoded())
        }
    }

    @Test
    fun `invalid encodings are no place`() {
        val cases = cases("invalid")
        assertTrue(cases.size >= 8)
        for (case in cases) assertNull(case.name(), Destination.decode(case.text("json")!!))
    }

    @Test
    fun `lenient encodings drop what they don't know`() {
        val cases = cases("lenient")
        assertTrue(cases.size >= 8)
        for (case in cases) {
            assertEquals(case.name(), case.text("reads"), Destination.decode(case.text("json")!!)?.encoded())
        }
    }

    @Test
    fun `every notification spelling still reads`() {
        val cases = cases("payloads")
        assertTrue(cases.size >= 15)
        for (case in cases) {
            val info = case["userInfo"]!!.jsonObject
            // Firebase data and intent extras are strings; a list arrives as its JSON.
            val extra = { key: String ->
                when (val value = info[key]) {
                    null, JsonNull -> null
                    is JsonPrimitive -> value.content
                    else -> value.toString()
                }
            }
            val parsed = DestinationPayloads.from(extra, case.text("thread") ?: "")
            assertEquals(case.name(), case.text("destination"), parsed?.encoded())
        }
    }

    @Test
    fun `every link still reads`() {
        val cases = cases("urls")
        assertTrue(cases.size >= 8)
        for (case in cases) {
            assertEquals(case.name(), case.text("destination"), DestinationPayloads.fromUrl(case.text("url")!!)?.encoded())
        }
    }

    @Test
    fun `the resolver agrees with every case`() {
        val cases = cases("resolve")
        assertTrue(cases.size >= 50)
        for (case in cases) {
            val destination = Destination.decode(case.text("destination")!!)
            assertNotNull(case.name(), destination)
            val arrival = DestinationResolver.Arrival.entries.first { it.wire == case.text("arrival") }
            val got = DestinationResolver.resolve(
                destination!!, arrival, world(case["world"]!!.jsonObject),
                elapsedMs = (case["elapsed"]!!.jsonPrimitive.double * 1000).toLong(),
                deadlineMs = (case["deadline"]!!.jsonPrimitive.double * 1000).toLong(),
                interrupted = case["interrupted"]?.jsonPrimitive?.boolean ?: false,
            )
            assertEquals(case.name(), expected(case["expect"]!!.jsonObject), got)
        }
    }

    private fun expected(json: JsonObject): DestinationResolver.Resolution {
        if (json["wait"]?.jsonPrimitive?.booleanOrNull == true) return DestinationResolver.Resolution.Wait
        json.text("connect")?.let { return DestinationResolver.Resolution.Connect(it) }
        json.text("open")?.let {
            return DestinationResolver.Resolution.Open(Destination.decode(it)!!, json["fellBack"]?.jsonPrimitive?.boolean ?: false)
        }
        val note = json.text("stay")?.let { wire -> DestinationResolver.Note.entries.first { it.wire == wire } }
        return DestinationResolver.Resolution.Stay(note)
    }

    private fun world(json: JsonObject): DestinationResolver.World {
        fun JsonObject.list(key: String) = (this[key] as? JsonArray)?.map { it.jsonObject }
        fun JsonObject.flag(key: String, default: Boolean) = (this[key] as? JsonPrimitive)?.booleanOrNull ?: default
        val seats = json.list("seats").orEmpty().map { seat ->
            DestinationResolver.World.Seat(
                host = seat.text("host")!!,
                runnerId = seat.text("runnerId"),
                ready = seat.flag("ready", false),
                idle = seat.flag("idle", false),
                workspaces = seat.list("workspaces")?.map {
                    DestinationResolver.World.Workspace(it.text("id")!!, it.flag("orchestrator", true))
                },
                worktrees = seat.list("worktrees")?.map { tree ->
                    DestinationResolver.World.Worktree(
                        tree.text("id")!!, tree.text("workspace"),
                        tree.list("terminals").orEmpty().map {
                            DestinationResolver.World.Terminal(it.text("id")!!, it.flag("orchestrator", false))
                        },
                    )
                },
                boards = (seat["boards"] as? JsonObject).orEmpty().mapValues { (_, rows) ->
                    rows.jsonArray.map { it.jsonObject }.map { row ->
                        DestinationResolver.World.Task(
                            row.text("id")!!, row.text("key"), row.text("repository"),
                            (row["tabs"] as? JsonArray)?.map { it.jsonPrimitive.content },
                        )
                    }
                },
                absentTasks = (seat["absentTasks"] as? JsonArray)?.map { it.jsonPrimitive.content }.orEmpty(),
            )
        }
        val last = json["lastWorkspace"] as? JsonObject
        return DestinationResolver.World(
            seats, last?.let { DestinationResolver.World.Last(it.text("host")!!, it.text("workspace")!!) },
        )
    }

    // ---- Kotlin's own ----

    @Test
    fun `typed values write the fixture's bytes`() {
        val byName = cases("encodings").associate { it.name() to it.text("json") }
        assertEquals(byName["needs you"], Destination.NEEDS_YOU.encoded())
        assertEquals(
            byName["a task, restored, with its tab, pane and agent"],
            Destination(
                Destination.Runner(host = "h1"), Destination.Place.Task("ws-a", Destination.TaskRef(id = "T1")),
                tab = Destination.Tab.CHANGES, pane = "t-1", agent = "t-2",
            ).encoded(),
        )
        assertEquals(
            byName["a slash and an accent are written as they are"],
            Destination(Destination.Runner(host = "ssh://box"), Destination.Place.Workspace("team/é")).encoded(),
        )
    }

    @Test
    fun `a local post's extras read back`() {
        val value = Destination(
            Destination.Runner(id = "r1"), Destination.Place.Task(null, Destination.TaskRef(key = "bil-7", repository = "repo-1")),
            question = true,
        )
        val extras = DestinationPayloads.extras(value)
        assertEquals(value, DestinationPayloads.from(extras::get))
        // The older spelling rides beside it, for a reader that doesn't know the encoding.
        assertEquals("bil-7", extras["task"])
        assertEquals("r1", extras["runner"])
    }

    @Test
    fun `the notice id parses as AgentKit's does`() {
        assertEquals("r1" to "bil-7", DestinationPayloads.parseNoticeId("t:r1:bil-7"))
        assertEquals("r1" to "a:b", DestinationPayloads.parseNoticeId("t:r1:a:b"))
        assertNull(DestinationPayloads.parseNoticeId("t:0123456789abcdef"))
        assertNull(DestinationPayloads.parseNoticeId("t::bil-7"))
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
