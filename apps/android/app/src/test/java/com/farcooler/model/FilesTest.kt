package com.farcooler.model

import com.farcooler.ui.Route
import com.farcooler.ui.location
import com.farcooler.ui.route
import java.io.File
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The phones' read-only Files browser (ov-259), minus its views: AgentKit's
 * `FilesBrowserTests`, against the same `test/fixtures/files-lines.json`.
 */
class FilesTest {
    private fun fixture(): JsonObject {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, "test/fixtures/files-lines.json")
            if (candidate.isFile) return Json.parseToJsonElement(candidate.readText()).jsonObject
            directory = directory.parentFile
        }
        throw AssertionError("no files-lines.json above ${System.getProperty("user.dir")}")
    }

    @Test
    fun `every line case in the shared fixture breaks as it says`() {
        val cases = fixture().getValue("lines").jsonArray
        assertTrue(cases.size >= 10)
        for (case in cases) {
            val text = case.jsonObject.getValue("text").jsonPrimitive.content
            val lines = case.jsonObject.getValue("lines").jsonArray.map { it.jsonPrimitive.content }
            assertEquals(text, lines, FilesText.lines(text))
        }
    }

    @Test
    fun `every display case in the shared fixture cuts where it says`() {
        val cases = fixture().getValue("display").jsonArray
        assertTrue(cases.size >= 6)
        for (case in cases.map { it.jsonObject }) {
            val text = case.getValue("prefix").jsonPrimitive.content +
                case.getValue("repeat").jsonPrimitive.content.repeat(case.getValue("count").jsonPrimitive.int)
            val shown = FilesText.display(text, case.getValue("limit").jsonPrimitive.int)
            val cut = case.getValue("cut").jsonPrimitive.boolean
            assertEquals("$case", cut, shown.cut)
            val keeps = case.getValue("keeps").jsonPrimitive.int
            assertEquals("$case", if (cut) text.substring(0, keeps) + "…" else text, shown.text)
        }
    }

    @Test
    fun `a size reads as Finder and the iPhone say it`() {
        assertEquals("12 bytes", FilesText.size(12))
        assertEquals("2 KB", FilesText.size(2_048))
        assertEquals("3.1 MB", FilesText.size(3_100_000))
        assertEquals("12 MB", FilesText.size(12_000_000))
    }

    @Test
    fun `a link leads where it points or nowhere`() {
        assertEquals("docs/v2/readme.md", FilesPaths.linkDestination("docs/latest", "v2/readme.md", ""))
        assertEquals("src/main.rs", FilesPaths.linkDestination("docs/latest", "../src/main.rs", ""))
        assertNull(FilesPaths.linkDestination("docs/latest", "../../etc/passwd", ""))
        // A phone never learns the runner's path, so an absolute target leads nowhere.
        assertNull(FilesPaths.linkDestination("latest", "/var/log/syslog", ""))
        assertEquals("src/a.rs", FilesPaths.linkDestination("latest", "/repo/src/a.rs", "/repo"))
    }

    @Test
    fun `failures are sentences, not words`() {
        assertEquals(FileReadFailure.MISSING, FileReadFailure.from("not-found", inFolder = false, atRoot = false))
        assertEquals(FileReadFailure.FOLDER_GONE, FileReadFailure.from("not-found", inFolder = true, atRoot = true))
        assertEquals(FileReadFailure.MISSING_IN_FOLDER, FileReadFailure.from("not-found", inFolder = true, atRoot = false))
        assertEquals(FileReadFailure.NOT_A_FILE, FileReadFailure.from("invalid-argument", false, false))
        assertEquals(FileReadFailure.RUNNER_TOO_OLD, FileReadFailure.from("capability-unsupported", true, true))
        assertEquals(FileReadFailure.FAILED, FileReadFailure.from(null, false, false))
        for (why in FileReadFailure.entries) {
            assertFalse(why.sentence.contains("-"))
            assertTrue(why.directorySentence.isNotEmpty())
        }
        assertFalse(FileReadFailure.MISSING.directorySentence == FileReadFailure.MISSING.sentence)
    }

    @Test
    fun `a state or kind a newer runner sends is still read`() {
        val read = FileRead.parse(Json.parseToJsonElement(
            """{"path":"a","state":"hologram","size":1,"text":"","linkTarget":""}""").jsonObject)
        assertEquals(FileRead.State.UNKNOWN, read.state)
        assertEquals(FileRead.State.TOO_LARGE, FileRead.parse(Json.parseToJsonElement(
            """{"path":"a","state":"too_large","size":9,"text":"","linkTarget":""}""").jsonObject).state)
        val dir = FileListing.parse(Json.parseToJsonElement(
            """{"path":"","truncated":false,"entries":[{"name":"x","kind":"socket","size":0,"linkTarget":""}]}""").jsonObject)
        assertEquals(FileEntry.Kind.OTHER, dir.entries[0].kind)
    }

    // ---- a screen's content, over a fake runner ----

    private fun entry(name: String, kind: FileEntry.Kind, size: Long = 0, to: String = "") =
        FileEntry(name, kind, size, to)

    private class FakeRunner : FilesSource {
        val dirs = HashMap<String, FileListing>()
        val files = HashMap<String, FileRead>()
        val listed = ArrayList<String>()
        val read = ArrayList<String>()

        override suspend fun list(place: FilesPlace, path: String): Result<FileListing> {
            listed += path
            return dirs[path]?.let { Result.success(it) }
                ?: Result.failure(FilesFailure(if (path in files) FileReadFailure.NOT_A_FILE else FileReadFailure.MISSING))
        }

        override suspend fun read(place: FilesPlace, path: String): Result<FileRead> {
            read += path
            return files[path]?.let { Result.success(it) }
                ?: Result.failure(FilesFailure(if (path in dirs) FileReadFailure.NOT_A_FILE else FileReadFailure.MISSING))
        }
    }

    private val wt = FilesPlace.Worktree("w1")

    private fun runner() = FakeRunner().apply {
        dirs[""] = FileListing(
            "", listOf(
                entry("src", FileEntry.Kind.DIRECTORY), entry("README.md", FileEntry.Kind.FILE, 2_048),
                entry("latest", FileEntry.Kind.LINK, to = "src/main.rs"),
                entry("out", FileEntry.Kind.LINK, to = "/var/out"), entry("fifo", FileEntry.Kind.OTHER),
            ), false,
        )
        dirs["src"] = FileListing("src", listOf(entry("main.rs", FileEntry.Kind.FILE, 12)), false)
        files["src/main.rs"] = FileRead("src/main.rs", FileRead.State.TEXT, 12, "fn main() {}\r\n}\n", "")
    }

    @Test
    fun `a directory lists rows that lead to the next screen`() = runTest {
        val content = FilesLoader.load(FilesLocation(wt), runner())
        val dir = (content as FilesContent.Directory).directory
        assertEquals(listOf("src", "README.md", "latest", "out", "fifo"), dir.rows.map { it.name })
        assertEquals(FilesLocation(wt, "src", FilesExpecting.DIRECTORY), dir.rows[0].destination)
        assertEquals(FilesLocation(wt, "README.md", FilesExpecting.FILE), dir.rows[1].destination)
        assertEquals(FilesText.size(2_048), dir.rows[1].detail)
        assertEquals("→ src/main.rs", dir.rows[2].detail)
        assertEquals(FilesLocation(wt, "src/main.rs", FilesExpecting.EITHER), dir.rows[2].destination)
        assertNull(dir.rows[3].destination)
        assertNull(dir.rows[4].destination)
        assertNull(dir.footer)
        assertNull(dir.empty)
    }

    @Test
    fun `a child screen reads only its own path and numbers the text by line`() = runTest {
        val fake = runner()
        val content = FilesLoader.load(FilesLocation(wt, "src/main.rs", FilesExpecting.FILE), fake)
        val code = (content as FilesContent.Code).code
        assertEquals(listOf("fn main() {}", "}"), code.lines)
        assertEquals(1, code.gutterDigits)
        assertFalse(code.anyCut)
        assertEquals(listOf("src/main.rs"), fake.read)
        assertTrue(fake.listed.isEmpty())
    }

    @Test
    fun `a file of one huge line is one short line`() = runTest {
        val huge = "x".repeat(512 * 1024)
        val fake = FakeRunner().apply { files["min.js"] = FileRead("min.js", FileRead.State.TEXT, huge.length.toLong(), huge, "") }
        val code = (FilesLoader.load(FilesLocation(wt, "min.js", FilesExpecting.FILE), fake) as FilesContent.Code).code
        assertEquals(1, code.lines.size)
        assertEquals(FilesText.LINE_LIMIT + 1, code.lines[0].length)
        assertTrue(code.anyCut)
    }

    @Test
    fun `two names that decode alike are two rows with two keys`() {
        val dir = FilesLoader.directory(
            FileListing("", listOf(entry("x\uFFFD", FileEntry.Kind.FILE), entry("x\uFFFD", FileEntry.Kind.FILE),
                entry("y", FileEntry.Kind.DIRECTORY)), false),
            FilesLocation(wt), "",
        )
        assertEquals(3, dir.rows.size)
        assertEquals(3, dir.rows.map { it.key }.toSet().size)
    }

    @Test
    fun `titles and a truncated or empty directory say so`() {
        assertEquals("logs", FilesLocation(FilesPlace.Folder("logs")).title)
        assertEquals("Files", FilesLocation(wt).title)
        assertEquals("lib", FilesLocation(wt, "src/lib").title)
        val cut = FilesLoader.directory(
            FileListing("", (0 until 5_000).map { entry("f$it", FileEntry.Kind.FILE) }, true), FilesLocation(wt), "")
        assertEquals("Showing the first 5,000 items.", cut.footer)
        assertEquals("This folder is empty.", FilesLoader.directory(FileListing("", emptyList(), false), FilesLocation(wt), "").empty)
    }

    @Test
    fun `binary and too-large files say how big they are and nothing else`() {
        val here = FilesLocation(wt, "a", FilesExpecting.FILE)
        val size = FilesText.size(12_000_000)
        assertEquals(FilesContent.Message("This is a binary file. It’s $size."),
            FilesLoader.file(FileRead("a", FileRead.State.BINARY, 12_000_000, "", ""), here, ""))
        assertEquals(FilesContent.Message("This file is too large to show here. It’s $size."),
            FilesLoader.file(FileRead("a", FileRead.State.TOO_LARGE, 12_000_000, "", ""), here, ""))
    }

    @Test
    fun `a link screen offers its destination only inside the place`() {
        val here = FilesLocation(FilesPlace.Folder("logs"), "current", FilesExpecting.FILE)
        assertEquals(
            FilesContent.Link("2026/a.log", FilesLocation(FilesPlace.Folder("logs"), "2026/a.log", FilesExpecting.EITHER)),
            FilesLoader.file(FileRead("current", FileRead.State.LINK, 0, "", "2026/a.log"), here, ""),
        )
        assertEquals(
            FilesContent.Link("/etc/hosts", null),
            FilesLoader.file(FileRead("current", FileRead.State.LINK, 0, "", "/etc/hosts"), here, ""),
        )
    }

    @Test
    fun `a link to a folder is read as a file first and then listed`() = runTest {
        val fake = runner()
        val content = FilesLoader.load(FilesLocation(wt, "src", FilesExpecting.EITHER), fake)
        assertEquals(listOf("main.rs"), (content as FilesContent.Directory).directory.rows.map { it.name })
        assertEquals(listOf("src"), fake.read)
        assertEquals(listOf("src"), fake.listed)
    }

    @Test
    fun `a failure is one sentence for the kind of screen`() = runTest {
        val fake = FakeRunner()
        assertEquals(FilesContent.Failed(FileReadFailure.MISSING.sentence),
            FilesLoader.load(FilesLocation(wt, "gone", FilesExpecting.FILE), fake))
        assertEquals(FilesContent.Failed(FileReadFailure.MISSING.directorySentence),
            FilesLoader.load(FilesLocation(wt, "gone", FilesExpecting.DIRECTORY), fake))
    }

    @Test
    fun `a call names a worktree or a folder never both, and a core refusal becomes a sentence`() = runTest {
        assertEquals(mapOf("worktree" to "w1", "path" to "src"), CoreFilesSource.arguments(wt, "src"))
        assertEquals(mapOf("folder" to "logs", "path" to ""), CoreFilesSource.arguments(FilesPlace.Folder("logs"), ""))
        val source = CoreFilesSource(
            call = { method, args ->
                if (method == CoreFilesSource.LIST_METHOD && "folder" !in args) {
                    Json.parseToJsonElement(
                        """{"path":"","truncated":false,"entries":[{"name":"a","kind":"file","size":3,"linkTarget":""}]}""").jsonObject
                } else throw IllegalStateException("refused")
            },
            refusalWord = { "not-found" },
        )
        assertEquals(listOf("a"), source.list(wt, "").getOrThrow().entries.map { it.name })
        assertEquals(FileReadFailure.FOLDER_GONE, (source.read(FilesPlace.Folder("logs"), "").exceptionOrNull() as FilesFailure).why)
        assertEquals(FileReadFailure.MISSING_IN_FOLDER, (source.read(FilesPlace.Folder("logs"), "a").exceptionOrNull() as FilesFailure).why)
    }

    @Test
    fun `a route and its location are one place`() {
        val location = FilesLocation(FilesPlace.Folder("logs"), "2026/a.log", FilesExpecting.EITHER)
        assertEquals(location, location.route("h").location())
        val wtLocation = FilesLocation(wt, "src")
        assertEquals(Route.Files("h", worktreeId = "w1", path = "src"), wtLocation.route("h"))
        assertEquals(wtLocation, wtLocation.route("h").location())
    }

    // ---- the doors ----

    private fun build(caps: Set<String>, scope: String = "control", folders: List<String>? = null) =
        DaemonBuild("1", true, "linux", capabilities = caps, grantedScope = scope, readOnlyFolders = folders)

    @Test
    fun `files are offered only where the runner serves them and the grant may read`() {
        assertTrue(build(setOf("worktree_files")).offersFiles)
        assertFalse("an older runner", build(setOf("tasks")).offersFiles)
        assertFalse("a read grant is refused by the runner", build(setOf("worktree_files"), scope = "read").offersFiles)
        assertTrue("no answer is not no permission", build(setOf("worktree_files"), scope = "unspecified").offersFiles)
        assertFalse("a runner older than capabilities", build(emptySet()).offersFiles)
    }

    @Test
    fun `folders are offered with their capability and at least one name`() {
        val both = setOf("worktree_files", "read_only_folders")
        assertEquals(listOf("logs", "notes"), build(both, folders = listOf("logs", "notes")).sharedFolders)
        assertTrue(build(both, folders = emptyList()).sharedFolders.isEmpty())
        assertTrue(build(both, folders = null).sharedFolders.isEmpty())
        assertTrue(build(setOf("worktree_files"), folders = listOf("logs")).sharedFolders.isEmpty())
        assertTrue(build(both, scope = "read", folders = listOf("logs")).sharedFolders.isEmpty())
    }
}
