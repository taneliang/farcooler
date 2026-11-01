package com.farcooler.model

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Where a ticket's or a card's pull request stands, worded as the Plan view does (ov-317): the iPhone's
 * `PlanPrStageTests`, rule for rule, in Material's sentence case. The bytes are `test/fixtures/plan-stages.json`, which
 * `plan_stages_json_is_the_shape_the_mac_reads` in `crates/client` writes from the runner's own wire messages, so the
 * decoder is fed what the runner sends; the phones' board is `plan-phones-ov317.json`.
 */
class PlanPrStageTest {
    private fun plan(file: String): Plan {
        val root = Json.parseToJsonElement(read("test/fixtures/$file")).jsonObject
        return Plan.decode(root["plan"]?.jsonObject ?: root)
    }

    private fun read(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            File(directory, relative).takeIf { it.isFile }?.let { return it.readText() }
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }

    private val stages = plan("plan-stages.json")
    private fun ticket(n: Int) = stages.tickets[n - 1]

    @Test
    fun `every stage kind the runner sends decodes and has words`() {
        assertEquals(
            listOf(
                "Building", "Agent review", "Fixing", "Waiting on alice and 2 others",
                "Waiting on maximilian-wolfgang-hohenzollern-schwarzenberg", "Changes requested",
                "Approved, checks running", "Checks failing", "Merge conflicts", "Approved",
                "Queued, position 2", "Merged", "Closed", "PR state unknown",
            ),
            (1..14).map { ticket(it).prStage?.words },
        )
        assertEquals("all thirteen kinds, none lost", 13, (1..14).mapNotNull { ticket(it).prStage?.kind }.toSet().size)
    }

    @Test
    fun `an attention stage leads with the problem and Approved stays in the state line`() {
        for ((n, problem) in listOf(8 to "Checks failing", 9 to "Merge conflicts")) {
            val stage = ticket(n).prStage!!
            assertEquals(problem, stage.words)
            assertEquals("Approved", stage.stateWords)
            assertEquals("Approved, ${problem.replaceFirstChar { it.lowercase() }}", stage.fullWords)
            assertEquals(problem, stage.rowWords)
            assertEquals("the gray line", "Approved", PlanWords.status(ticket(n), stage = false))
            assertEquals("a line with room for one phrase", "Approved, ${problem.replaceFirstChar { it.lowercase() }}", PlanWords.status(ticket(n)))
        }
        assertNull("changes requested is its own state", ticket(6).prStage?.stateWords)
        assertEquals("Changes requested", ticket(6).prStage?.rowWords)
        assertEquals("Waiting on alice and 2 others", ticket(4).prStage?.rowWords)
    }

    @Test
    fun `only changes requested, failing checks and conflicts need a person`() {
        assertEquals(listOf(6, 8, 9), (1..14).filter { ticket(it).prStage?.needsAttention == true })
    }

    @Test
    fun `the facts under the words come through`() {
        val stage = ticket(4).prStage!!
        assertEquals(listOf("alice", "bob", "org/web"), stage.reviewers)
        assertEquals(31, stage.prNumber)
        assertEquals(2, stage.unresolvedThreads)
        assertEquals("PR 31 · 2 threads open", stage.detail)
        assertEquals("https://github.com/acme/app/pull/31", stage.link)
        assertEquals("failing", ticket(8).prStage?.checks)
        assertEquals("no threads counted, none said", "PR 32", ticket(5).prStage?.detail)
    }

    @Test
    fun `unknown is added to the state and never replaces it, and building says nothing twice`() {
        assertEquals("In review · PR state unknown", PlanWords.status(ticket(14)))
        assertEquals("In review", PlanWords.status(ticket(1)))
        assertNull(ticket(1).shownStage)
        assertEquals("Waiting on alice and 2 others", PlanWords.status(ticket(4)))
        assertEquals("Changes requested", PlanWords.status(ticket(6)))
        assertEquals("In review", PlanWords.status(ticket(6), stage = false))
        assertEquals("Queued, position 2", PlanWords.status(ticket(11)))
    }

    @Test
    fun `a ticket told nothing reads as it always did, and a finished ticket ignores a stale stage`() {
        val silent = stages.tickets.last()
        assertNull(silent.prStage)
        assertNull(silent.card.stage)
        assertEquals("In review", PlanWords.status(silent))
        val landed = ticket(4).copy(stage = TicketStage.LANDED)
        assertNull(landed.shownStage)
        assertEquals("Landed", PlanWords.status(landed))
    }

    @Test
    fun `a kind this build does not know falls back to the runner's words, or says nothing`() {
        val newer = PlanPrStage.decode(Json.parseToJsonElement("""{"kind":"needs_a_dance","label":"Awaiting the council"}"""))!!
        assertEquals(PlanPrStageKind.UNSPECIFIED, newer.kind)
        assertEquals("Awaiting the council", newer.words)
        val silent = PlanPrStage.decode(Json.parseToJsonElement("""{"kind":"needs_a_dance"}"""))!!
        assertNull(silent.words)
        assertNull(silent.shown)
        assertNull("a runner before the stage", PlanPrStage.decode(null))
    }

    @Test
    fun `waiting on names the login, a couple, or a count`() {
        assertEquals("Waiting on a reviewer", PlanPrStage.waiting(emptyList()))
        assertEquals("Waiting on alice", PlanPrStage.waiting(listOf("alice")))
        assertEquals("Waiting on alice and bob", PlanPrStage.waiting(listOf("alice", "bob")))
        assertEquals("Waiting on alice and 2 others", PlanPrStage.waiting(listOf("alice", "bob", "carol")))
    }

    @Test
    fun `Mark ready is for a draft the runner numbered and nothing else`() {
        val phones = plan("plan-phones-ov317.json")
        assertEquals(listOf("ov-2", "ov-3"), phones.tickets.filter { it.prStage?.canMarkReady == true }.map { it.key })
        assertFalse("building has no pull request", phones.tickets.first { it.key == "ov-1" }.prStage!!.canMarkReady)
        assertFalse("a pull request that could not be read", phones.tickets.first { it.key == "ov-14" }.prStage!!.canMarkReady)
        assertFalse("a draft with no number to name", phones.tickets.first { it.key == "ov-2" }.prStage!!.copy(prNumber = 0).canMarkReady)
        assertEquals(
            listOf(PlanPrStageKind.AGENT_REVIEW, PlanPrStageKind.FIXING),
            PlanPrStageKind.entries.filter { PlanPrStage(it, prNumber = 7).isDraft },
        )
        // Each ticket's card carries its own stage, which the ticket's page draws.
        assertEquals("Agent review", phones.tickets.first { it.key == "ov-2" }.card.stage?.words)
    }

    @Test
    fun `a link is opened only when it is github dot com`() {
        assertEquals("https://github.com/acme/billing/pull/1#discussion_r1", GitHubLink.url("https://github.com/acme/billing/pull/1#discussion_r1"))
        assertEquals("https://GitHub.com/a/b/pull/1", GitHubLink.url("https://GitHub.com/a/b/pull/1"))
        for (bad in listOf(null, "", "http://github.com/a", "https://github.com.evil.example/a", "https://github.com@evil.example/a", "https://github.com:8443/a", "https://git hub.com/a", "https://github.com\\@evil.example/a", "ftp://github.com/a")) {
            assertNull("$bad", GitHubLink.url(bad))
        }
        assertTrue(GitHubLink.url("https://github.com") != null)
    }
}
