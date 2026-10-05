package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.File

/**
 * Every page fixture through Android's reader (ov-269 design 7, ov-285): the
 * documents the runner takes, what it stores of them, and each one it refuses.
 * The runner is the gate and the reader the second one (the fix-round rulings
 * on ov-284), so a refused document that reaches the phone anyway draws what
 * fits, says the rest didn't, never opens a link that isn't `https`, and never
 * stops the page drawing. AgentKit's `PageConformanceTests` holds the Mac's and
 * the iPhone's reader to the same files.
 */
class PageConformanceTest {
    private val root: File = run {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, "test/fixtures/pages")
            if (candidate.isDirectory) return@run candidate
            directory = directory.parentFile
        }
        throw AssertionError("Could not find test/fixtures/pages above ${System.getProperty("user.dir")}.")
    }

    /** Every `.json` under `test/fixtures/pages/` but the refusals' index, by its path there. */
    private val fixtures: List<String> = root.walkTopDown().filter { it.isFile && it.extension == "json" && it.name != "refusals.json" }
        .map { it.relativeTo(root).path }.sorted().toList()

    private fun read(name: String) = PageDoc.decode(File(root, name).readText())

    private fun tooLarge(doc: PageDoc) = doc.blocks.lastOrNull() == PageBlock.Unknown("too-large", PageWords.TOO_LARGE)

    /** The refusals' index: which file breaks which cap. */
    private val caps: Map<String, String> = (Json.parseToJsonElement(File(root, "refusals.json").readText()) as JsonArray)
        .mapNotNull { row ->
            val o = row.jsonObject
            val cap = o["cap"]?.jsonPrimitive?.contentOrNull ?: return@mapNotNull null
            o["file"]!!.jsonPrimitive.content to cap
        }.toMap()

    private fun refs(doc: PageDoc) = doc.blocks.sumOf { block ->
        when (block) {
            is PageBlock.Table -> block.rows.flatten().count { it.ref != null }
            is PageBlock.ListBlock -> block.items.count { it.ref != null }
            is PageBlock.Timeline -> block.entries.count { it.ref != null }
            is PageBlock.Links -> block.refs.size
            is PageBlock.Stats -> block.items.count { it.ref != null }
            else -> 0
        }
    }

    @Test
    fun `the reader reads every fixture, all but the one that isn't an object`() {
        assertEquals("a fixture came or went", 73, fixtures.size)
        for (name in fixtures) {
            if (name == "refused/not-an-object.json") {
                try {
                    read(name)
                    fail("a document that isn't an object decoded")
                } catch (expected: IllegalArgumentException) {
                }
                continue
            }
            val doc = read(name)
            assertTrue("$name drew nothing", doc.blocks.isNotEmpty() || name == "refused/no-blocks.json")
        }
    }

    @Test
    fun `a page listed with a document that isn't an object keeps its row`() {
        val raw = File(root, "refused/not-an-object.json").readText()
        val page = BoardPage.decode(Json.parseToJsonElement("""{"id":"p","slot":"s","title":"Kept","doc":$raw}""").jsonObject)
        assertEquals("Kept", page.title)
        assertNull(page.doc)
    }

    @Test
    fun `every size cap the runner refuses, the reader holds, then the too-large row`() {
        assertEquals("a cap came or went", 16, caps.size)
        for ((file, cap) in caps) {
            val doc = read(file)
            assertTrue("$file ($cap) drew without saying it's too large", tooLarge(doc))
            assertTrue(file, doc.blocks.size <= PageCaps.BLOCKS + 1)
            assertTrue(file, doc.title.length <= PageCaps.TITLE && doc.summary.length <= PageCaps.SUMMARY)
            assertTrue(file, (doc.glance ?: "").length <= PageCaps.GLANCE)
            assertTrue(file, refs(doc) <= PageCaps.REFS)
        }
    }

    @Test
    fun `the caps by value, 60 title characters, 32 KiB, 200 references`() {
        val title = read("refused/title-chars.json").title
        assertEquals(60, title.length)
        assertTrue(title.endsWith("…"))
        // Twenty 2,000-character paragraphs: sixteen fit in 32 KiB, and the
        // seventeenth would pass it.
        assertEquals(17, read("refused/document-bytes.json").blocks.size)
        val refs = read("refused/refs.json")
        assertEquals(200, refs(refs))
        val items = (refs.blocks[4] as PageBlock.ListBlock).items
        assertTrue("the words of a reference past the cap went", items.all { it.text.isNotEmpty() })
        assertTrue(items.any { it.ref == null })
    }

    @Test
    fun `a document inside the caps isn't marked and keeps every block`() {
        for (name in fixtures.filter { it.startsWith("normalized/") || !it.contains('/') }) {
            val doc = read(name)
            assertFalse(name, tooLarge(doc))
            val raw = Json.parseToJsonElement(File(root, name).readText()) as JsonObject
            assertEquals("$name lost a block", (raw["blocks"] as JsonArray).size, doc.blocks.size)
        }
    }

    @Test
    fun `no link the runner refuses opens, in a reference or in Markdown`() {
        val world = PageWorld()
        val linky = fixtures.filter { it.startsWith("refused/") && (it.contains("link") || it.startsWith("refused/md-")) }
        assertEquals(16, linky.size)
        for (name in linky) {
            for (block in read(name).blocks) {
                when (block) {
                    is PageBlock.Links -> block.refs.forEach { assertNull("$name opens ${it.target.rawName}", world.resolve(it).destination) }
                    is PageBlock.Text -> PageMarkdown.pieces(block.md).forEach { piece ->
                        val text = when (piece) {
                            is PageMarkdown.Piece.Prose -> piece.text
                            is PageMarkdown.Piece.Item -> piece.text
                            is PageMarkdown.Piece.Plain -> return@forEach
                        }
                        PageMarkdown.inline(text).mapNotNull { it.span.link }.forEach { assertNotNull("$name links $it", PageLinks.https(it)) }
                    }
                    else -> Unit
                }
            }
        }
    }

    @Test
    fun `a link whose words name another domain is followed by the domain it goes to`() {
        for (name in listOf("refused/md-label-names-another-domain.json", "refused/md-label-url-names-another-domain.json")) {
            val md = (read(name).blocks.single() as PageBlock.Text).md
            val runs = PageMarkdown.pieces(md).flatMap { (it as? PageMarkdown.Piece.Prose)?.let { p -> PageMarkdown.inline(p.text) }.orEmpty() }
            assertTrue("$name hides where it goes: $runs", runs.any { it.domain && it.span.text == " evil.example" })
        }
    }

    @Test
    fun `a block this build doesn't know is its alt, or says it needs a newer Far Cooler`() {
        assertEquals(listOf(PageBlock.Heading("Known"), PageBlock.Unknown("gauge", "Disk is 80% full.")), read("refused/future-version.json").blocks)
        assertEquals(listOf(PageBlock.Unknown("diagram", null)), read("refused/unknown-block.json").blocks)
    }
}
