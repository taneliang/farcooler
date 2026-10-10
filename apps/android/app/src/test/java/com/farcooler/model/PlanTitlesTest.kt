package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Lanes and trains named by what they do (ov-462), and a train's own
 * integrating agent (ov-461), against `test/fixtures/plan.json`. AgentKit's
 * `PlanTitlesTests` holds the Mac and the iPhone to the same file and words.
 */
class PlanTitlesTest {
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
    fun `a lane's title comes first and its slug second, and a lane with none shows its name`() {
        assertEquals(listOf("Mac follow-ups", "Mac interface polish", "fix-ac84"), plan.lanes.map { it.heading })
        assertEquals("mac-ux", plan.lanes[1].slug)
        assertNull(plan.lanes[2].slug)
        assertEquals("same", PlanLane("i", "same", LaneState.BUILDING, "", null, "", "", "", "", null, null, 0, false, 0, emptyList(), emptyList(), PlanSpend(), title = "").heading)
    }

    @Test
    fun `a train reads Train N, what it carries, its agent and its card`() {
        val train = plan.trains.single()
        assertEquals("Train 9", train.heading)
        assertEquals("integ-9", train.slug)
        assertEquals("Mac interface polish", train.carries)
        assertEquals("ov-2", train.card?.key)
        val agent = train.agent!!
        assertTrue(agent.isWorking)
        assertEquals("i1", agent.agentId)
        assertEquals(120_000L, agent.spend.totalTokens)
        assertTrue(TrainWords.train(train, plan.ciOf(train)).contains(" · Agent working · 120K tokens"))
    }

    @Test
    fun `a train's number is read off integ-N or train-N`() {
        assertEquals(72, TrainWords.number("integ-72"))
        assertEquals(5, TrainWords.number("Train-5"))
        assertNull(TrainWords.number("integ-x"))
        assertNull(TrainWords.number("rc-1"))
        assertNull(TrainWords.number("train-"))
        val bare = PlanTrain("i", "spring", "", null, TrainState.GATING, 0, emptyList(), "")
        assertEquals("spring", bare.heading)
        assertNull(bare.slug)
        assertEquals("Train 72", bare.copy(name = "integ-72").heading)
        assertEquals("Phones catch up", bare.copy(name = "train-3", title = "Phones catch up").heading)
    }

    @Test
    fun `a lane named like a live train is drawn by the train once`() {
        val object_ = repositoryFile("test/fixtures/plan.json")
        val withShadow = Plan.decode(object_).let { p ->
            val shadow = p.lanes[1].copy(id = "lane-shadow", name = "integ-9")
            p.copy(lanes = p.lanes + shadow, laneIsTrain = listOf(PlanFlaggedLane("lane-shadow", "integ-9")))
        }
        assertEquals(listOf(listOf("mac-ux")), withShadow.nowGroups.map { g -> g.lanes.map { it.name } })
        assertTrue(Plan.decode(object_).laneIsTrain.isEmpty())
    }
}
