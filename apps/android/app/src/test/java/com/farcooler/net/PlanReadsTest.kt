package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.model.PlanPage
import com.farcooler.model.PlanReadState
import com.farcooler.model.WorkspaceSummary
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** What a connection does with the plan layer (ov-274): the states a read lands in, and when it reads again. */
class PlanReadsTest {
    private val workspace = WorkspaceSummary(id = "w1", name = "Main", repository = "r1", isMain = true)
    private val plan: JsonObject = Json.parseToJsonElement(repositoryFile("test/fixtures/plan.json")).jsonObject

    @Test
    fun `a board's plan is read for its workspace and kept`() = runBlocking {
        val sent = mutableListOf<Pair<String, JsonObject>>()
        val reads = PlanReads({ method, args -> sent += method to args; plan }, runnerCan = { true })
        reads.read(workspace)
        assertTrue(reads.states.value["w1"] is PlanReadState.Loaded)
        assertEquals("plan.get", sent.single().first)
        assertEquals("w1", sent.single().second["workspace"]?.toString()?.trim('"'))
    }

    @Test
    fun `a runner without board_plan is never asked`() = runBlocking {
        val reads = PlanReads({ _, _ -> error("asked") }, runnerCan = { false })
        reads.read(workspace)
        assertEquals(PlanReadState.NeedsUpdate, reads.states.value["w1"])
    }

    @Test
    fun `a refusal, and a read nobody answers, are unavailable`() = runBlocking {
        val refused = PlanReads({ _, _ -> throw CoreException("no", "unavailable") }, runnerCan = { true })
        refused.read(workspace)
        assertEquals(PlanReadState.Unavailable, refused.states.value["w1"])
        val hung = PlanReads({ _, _ -> delay(60_000); plan }, runnerCan = { true }, timeoutMs = 80)
        hung.read(workspace)
        assertEquals(PlanReadState.Unavailable, hung.states.value["w1"])
    }

    @Test
    fun `a refusal that names the missing capability is an update to ask for`() = runBlocking {
        val reads = PlanReads({ _, _ -> throw CoreException("no", "capability-unsupported") }, runnerCan = { null })
        reads.read(workspace)
        assertEquals(PlanReadState.NeedsUpdate, reads.states.value["w1"])
    }

    @Test
    fun `a failed read over a plan in hand keeps the plan`() = runBlocking {
        var fail = false
        val reads = PlanReads({ _, _ -> if (fail) throw CoreException("no") else plan }, runnerCan = { true })
        reads.read(workspace)
        fail = true
        reads.read(workspace)
        assertTrue(reads.states.value["w1"] is PlanReadState.Loaded)
    }

    @Test
    fun `a plan notice reads again only a board whose plan was asked for`() = runBlocking {
        var reads = 0
        val plans = PlanReads({ _, _ -> reads++; plan }, runnerCan = { true })
        val other = WorkspaceSummary(id = "w2", name = "Billing", repository = "r1")
        plans.heard("w1", listOf(workspace, other))
        assertEquals("nothing asked for yet, nothing read", 0, reads)
        plans.read(workspace)
        plans.heard("W1", listOf(workspace, other))
        assertEquals(2, reads)
        plans.heard("w2", listOf(workspace, other))
        assertEquals("a board nobody asked about stays unread", 2, reads)
    }

    @Test
    fun `a record is read for its page, and one that fails leaves the page without`() = runBlocking {
        val sent = mutableListOf<JsonObject>()
        val record = Json.parseToJsonElement("""{"events":[{"at":1,"actor":"m","kind":"state","body":"Started."}]}""").jsonObject
        val plans = PlanReads({ method, args -> sent += args; if (method == "plan.events") record else plan }, runnerCan = { true })
        plans.readRecord(PlanPage.Lane("l1"))
        assertEquals("l1", sent.single()["lane"]?.toString()?.trim('"'))
        assertEquals(1, plans.records.value[PlanPage.Lane("l1")]?.events?.size)
        val broken = PlanReads({ _, _ -> throw CoreException("no") }, runnerCan = { true })
        broken.readRecord(PlanPage.Theme("t1"))
        assertTrue(broken.records.value.isEmpty())
    }

    @Test
    fun `a runner without board_plan is not asked for a record either`() = runBlocking {
        var asked = 0
        val plans = PlanReads({ _, _ -> asked++; JsonObject(emptyMap()) }, runnerCan = { false })
        plans.readRecord(PlanPage.Theme("t1"))
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
