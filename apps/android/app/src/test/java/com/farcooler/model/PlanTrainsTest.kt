package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Trains (ov-309) and the runner's CI reads (ov-306) on Android, against
 * `test/fixtures/plan.json`, the Rust client's own output; and a page's live
 * references against `test/fixtures/pages/normalized/live.json`, what the
 * runner stores. AgentKit's `PlanTrainsTests` and `PageLiveDataTests` hold
 * the Mac and the iPhone to the same files and the same words.
 */
class PlanTrainsTest {
    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }

    private val plan = Plan.decode(repositoryFile("test/fixtures/plan.json"))

    @Test
    fun `the plan carries trains, CI reads, a theme's spend and the board's counts`() {
        val train = plan.trains.single()
        assertEquals("integ-9", train.name)
        assertEquals(TrainState.RED, train.state)
        assertEquals("c85bf83d", train.pushedSha)
        assertEquals(listOf(PlanTrainLane(plan.lanes[1].id, "mac-ux")), train.lanes)
        val read = assertNotNullOf(plan.ciOf(train))
        assertEquals(CiStatus.FAILED, read.status)
        assertEquals(3, read.jobs.size)
        assertEquals(320_000L, plan.themes.single().spend?.totalTokens)
        assertEquals(9, plan.cardCount("open"))
        assertEquals(3, plan.cardCount("in_review"))
    }

    @Test
    fun `Now heads a train's lanes with the train, and a lane says it's in it`() {
        val groups = plan.nowGroups
        assertEquals(listOf("integ-9"), groups.map { it.train?.name })
        assertEquals(listOf(listOf("mac-ux")), groups.map { g -> g.lanes.map { it.name } })
        assertEquals("Red · c85bf83d · CI Failed · 1 of 3 jobs failed", TrainWords.train(groups[0].train!!, plan.ciOf(groups[0].train!!)))
        assertTrue(TrainWords.needsAttention(groups[0].train!!, null))
        assertEquals("In review · in integ-9", PlanWords.status(plan.lanes[1]))
    }

    @Test
    fun `an older answer with no trains still reads`() {
        val old = Plan.decode("""{"now_ms": 1, "themes": [], "lanes": [], "order": [], "cards": []}""")
        assertTrue(old.trains.isEmpty() && old.ci.isEmpty())
        assertNull(old.boardCounts)
        assertNull(old.cardCount("open"))
    }

    @Test
    fun `CI words, in sentence case`() {
        val jobs = listOf(PlanCiJob("a", "running"), PlanCiJob("b", "passed"))
        assertEquals("Running · 1 of 2 jobs done", TrainWords.ciSummary(PlanCiRead("main", status = CiStatus.RUNNING, jobs = jobs)))
        assertEquals("No runs yet", TrainWords.ciSummary(PlanCiRead("main", status = CiStatus.NONE)))
        assertEquals("CI unknown", TrainWords.ciSummary(PlanCiRead("main", status = CiStatus.UNKNOWN)))
    }

    private val live = PageDoc.decode(repositoryFile("test/fixtures/pages/normalized/live.json"))

    @Test
    fun `figures on the live page draw from the plan, failed CI in amber`() {
        val stats = live.blocks[1] as PageBlock.Stats
        assertEquals(PageRef(PageTarget.Ci("main")), stats.items[0].ref)
        assertEquals("", stats.items[0].value)
        val world = PageWorld(plan = plan)
        val shown = stats.items.map(world::statText)
        // Main isn't read in the fixture: its name, as plain text.
        assertEquals("Main", shown[0].value)
        assertEquals(PageWorld.StatShown("Failed", "pushed at midnight", PageTone.ATTENTION), shown[1])
        assertEquals("3", shown[2].value)
        assertEquals("9", shown[3].value)
        assertEquals("320K", shown[4].value)
        assertEquals("470K", shown[5].value)
    }

    @Test
    fun `a CI reference opens its run and says how it stands, and a count is a number`() {
        val world = PageWorld(plan = plan)
        val ci = world.resolve(PageRef(PageTarget.Ci("c85bf83d")))
        assertEquals("c85bf83d", ci.name)
        assertEquals("Failed · 1 of 3 jobs failed", ci.status)
        assertEquals(PageTone.ATTENTION, ci.statusTone)
        assertEquals(PageDestination.Url("https://github.com/taneliang/farcooler/actions/runs/37275435256"), ci.destination)
        assertEquals("11", world.cellText(PageCell(ref = PageRef(PageTarget.Cards("done")))))
        assertEquals("320K", world.cellText(PageCell(ref = PageRef(PageTarget.Theme("Visual language")), show = PageShow.SPEND)))
        val bare = PageWorld()
        assertEquals(PageResolved("Run 37275435256"), bare.resolve(PageRef(PageTarget.Ci("run:37275435256"))))
        assertEquals(PageResolved("In review"), bare.resolve(PageRef(PageTarget.Cards("in_review"))))
        assertFalse(bare.resolve(PageRef(PageTarget.Ci("main"))).destination != null)
    }

    private fun <T> assertNotNullOf(value: T?): T {
        assertNotNull(value)
        return value!!
    }
}
