package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.model.PlanPage
import com.farcooler.model.WorkspaceSummary
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * What a connection does with orchestrator pages (ov-285): one `page.list` per
 * board, answered here with the CLI's own `page list --json` for a seeded board
 * (`test/fixtures/pages-seeded.json`), the states a read lands in, and when it
 * reads again.
 */
class PageReadsTest {
    private val workspace = WorkspaceSummary(id = "w1", name = "Main", repository = "r1", isMain = true)
    private val list: JsonObject =
        Json.parseToJsonElement(repositoryFile("test/fixtures/pages-seeded.json")).jsonObject["pages"]!!.jsonObject

    @Test
    fun `a board's pages are read with their documents and kept`() = runBlocking {
        val sent = mutableListOf<Pair<String, JsonObject>>()
        val reads = PageReads({ method, args -> sent += method to args; list }, runnerCan = { true })
        reads.read(workspace)
        assertEquals("page.list", sent.single().first)
        assertEquals("w1", sent.single().second["workspace"]?.toString()?.trim('"'))
        val pages = reads.pages("w1")
        assertEquals(listOf("train", "spend", "risks"), pages.map { it.slot })
        assertTrue(pages.all { it.doc != null })
    }

    @Test
    fun `a runner without board_pages is never asked, and shows nothing`() = runBlocking {
        val reads = PageReads({ _, _ -> error("asked") }, runnerCan = { false })
        reads.read(workspace)
        assertNull(reads.lists.value["w1"])
        val notRead = PageReads({ _, _ -> error("asked") }, runnerCan = { null })
        notRead.read(workspace)
        assertNull("a runner whose build isn't read yet offers no pages", notRead.lists.value["w1"])
    }

    @Test
    fun `a refusal and a read nobody answers say so, and a failure over a list in hand keeps the list`() = runBlocking {
        val refused = PageReads({ _, _ -> throw CoreException("no", "unavailable") }, runnerCan = { true })
        refused.read(workspace)
        assertEquals(PageListState.Unavailable, refused.lists.value["w1"])
        val hung = PageReads({ _, _ -> delay(60_000); list }, runnerCan = { true }, timeoutMs = 80)
        hung.read(workspace)
        assertEquals(PageListState.Unavailable, hung.lists.value["w1"])
        var fail = false
        val kept = PageReads({ _, _ -> if (fail) throw CoreException("no") else list }, runnerCan = { true })
        kept.read(workspace)
        fail = true
        kept.read(workspace)
        assertEquals(3, kept.pages("w1").size)
    }

    @Test
    fun `a pages notice reads again a board whose pages were read, only in front`() = runBlocking {
        var reads = 0
        val pages = PageReads({ _, _ -> reads++; list }, runnerCan = { true })
        val other = WorkspaceSummary(id = "w2", name = "Billing", repository = "r1")
        val line = Json.parseToJsonElement("""{"event":"pages","workspace":"W1","slot":"train","removed":false}""").jsonObject
        pages.noticed(line, listOf(workspace, other), foreground = true)
        assertEquals("nothing read yet, nothing read again", 0, reads)
        pages.read(workspace)
        pages.noticed(line, listOf(workspace, other), foreground = false)
        assertEquals(1, reads)
        pages.noticed(Json.parseToJsonElement("""{"event":"plan","workspace":"w1"}""").jsonObject, listOf(workspace), foreground = true)
        assertEquals("a plan notice isn't page news", 1, reads)
        pages.noticed(line, listOf(workspace, other), foreground = true)
        assertEquals(2, reads)
    }

    @Test
    fun `a page has no record to read`() = runBlocking {
        var asked = 0
        val plans = PlanReads({ _, _ -> asked++; JsonObject(emptyMap()) }, runnerCan = { true })
        plans.readRecord(PlanPage.Page("train"))
        assertEquals(0, asked)
    }

    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }
}
