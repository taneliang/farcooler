package com.farcooler.model

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The front door's wire decode, its merge across runners, and its ordering.
 *
 * All three are pure, which is why they live in `model/` rather than inside the
 * composable — a phone can prove none of this and a JVM can prove all of it.
 * What is deliberately NOT here is anything about layout: no emulator or device
 * was available for this work.
 *
 * The rank numbers below are real ones. `farcooler_core::feed::rank` is
 * `tier * 100_000_000 + (100_000_000 - 1 - age_seconds)`, with blocked in tier
 * 0 and done-or-failed in tier 1, so a blocked agent's rank is always about
 * 10⁸ below a finished one's and an OLDER state inside a tier gets the SMALLER
 * number. Writing them out rather than naming them is the point: these tests
 * pass only if this app reads that arithmetic the way the host wrote it.
 */
class NeedsYouTest {
    // ---- the wire ----

    /** The same configuration `Connection` decodes with. */
    private val json = Json { ignoreUnknownKeys = true }

    /**
     * Transcribed key for key from `Session::changes_inbox` in
     * `crates/client/src/session.rs`, including the three keys this app
     * deliberately does not read and the top-level `elsewhere`. A rename on
     * either end fails here rather than going quiet on a phone as a permanently
     * zero count.
     *
     * Values are non-default throughout: `false` where the default is `false`
     * is a test that passes when the decode does nothing at all.
     */
    private val payload = """
        {
          "items": [
            {
              "workspace_id": "8f14e45f-ce5b-4a5e-9c2b-000000000001",
              "short": "8f14e4",
              "task_name": "Widen the model",
              "branch": "feat/widen-the-model",
              "changed_since_reviewed": true,
              "insertions": 82,
              "deletions": 13
            }
          ],
          "elsewhere": 0
        }
    """.trimIndent()

    @Test
    fun `every key the inbox sends is decoded under the name this app reads it by`() {
        val reply = json.decodeFromString(InboxReply.serializer(), payload)
        assertEquals(1, reply.items.size)
        val row = reply.items[0]
        assertEquals("8f14e45f-ce5b-4a5e-9c2b-000000000001", row.workspaceId)
        assertTrue("changed_since_reviewed must survive the snake_case", row.changedSinceReviewed)
        assertEquals(82, row.insertions)
        assertEquals(13, row.deletions)
        assertTrue(row.hasDiff)
    }

    /**
     * A daemon that answers with nothing but the id — every other field is
     * `#[serde(default)]`-shaped on the wire, and a proto-3 zero is simply
     * absent. This must be a clean, empty row rather than a decode failure that
     * costs the whole poll.
     */
    @Test
    fun `a row with only an id decodes to a clean empty one`() {
        val reply = json.decodeFromString(
            InboxReply.serializer(),
            """{"items":[{"workspace_id":"w"}]}""",
        )
        val row = reply.items[0]
        assertFalse(row.changedSinceReviewed)
        assertEquals(0, row.insertions)
        assertEquals(0, row.deletions)
        assertFalse("no numbers is no diff", row.hasDiff)
    }

    /** A newer daemon adding a key must not cost an older app its counts. */
    @Test
    fun `an unknown key is ignored rather than fatal`() {
        val reply = json.decodeFromString(
            InboxReply.serializer(),
            """{"items":[{"workspace_id":"w","insertions":3,"conflicts":7}],"nudges":1}""",
        )
        assertEquals(3, reply.items[0].insertions)
    }

    /** A reply with no items at all is empty, not a failure. */
    @Test
    fun `an empty inbox decodes`() {
        assertTrue(json.decodeFromString(InboxReply.serializer(), "{}").items.isEmpty())
    }

    /** Deletions alone are still a diff. A file emptied is not a clean worktree. */
    @Test
    fun `deletions alone count as a diff`() {
        assertTrue(InboxRow("w", insertions = 0, deletions = 4).hasDiff)
    }

    // ---- what gets a section ----

    @Test
    fun `only agents that want attention put a workspace on the front door`() {
        val sections = needsYou(
            listOf(
                input("a", workspace("w1", terminals = listOf(agent("t1", "working", 250_000_000)))),
                input("a", workspace("w2", terminals = listOf(agent("t2", "idle", 350_000_000)))),
                input("a", workspace("w3", terminals = listOf(agent("t3", "blocked", 99_999_399)))),
            )
        )
        assertEquals(listOf("w3"), sections.map { it.workspace.id })
    }

    /**
     * The exact case `e480559` added on iOS: an agent that finished and touched
     * no files. Nothing else brings its workspace here, and the owner's words
     * are that an answer arriving is the thing this screen most needs to say.
     */
    @Test
    fun `a finished agent gets a section even with no diff behind it`() {
        val sections = needsYou(
            listOf(
                input(
                    "a",
                    workspace("w1", terminals = listOf(agent("t1", "done", 199_999_989))),
                    counts = InboxRow("w1", changedSinceReviewed = false),
                )
            )
        )
        assertEquals(1, sections.size)
        assertEquals(listOf("t1"), sections[0].finished.map { it.id })
        assertTrue(sections[0].blocked.isEmpty())
    }

    @Test
    fun `hidden workspaces are not on the front door however loudly they ask`() {
        val sections = needsYou(
            listOf(
                input(
                    "a",
                    workspace(
                        "w1",
                        state = "hidden",
                        terminals = listOf(agent("t1", "blocked", 99_999_399)),
                    ),
                )
            )
        )
        assertTrue(sections.isEmpty())
    }

    /**
     * Both halves of the second tier, and they are not the same condition.
     * `hasDiff` stays true after you have read it; `changedSinceReviewed` is
     * what makes this an inbox rather than a list of every branch in flight.
     */
    @Test
    fun `an unread diff needs both a change and a diff`() {
        fun only(counts: InboxRow?) = needsYou(listOf(input("a", workspace("w1"), counts)))

        assertEquals(1, only(InboxRow("w1", true, 5, 1)).size)
        assertTrue("reviewed", only(InboxRow("w1", false, 5, 1)).isEmpty())
        assertTrue("nothing changed", only(InboxRow("w1", true, 0, 0)).isEmpty())
        assertTrue("never answered", only(null).isEmpty())
    }

    // ---- ordering ----

    /**
     * Blocked above done, and this app does not arrange it. Both agents are
     * `wantsAttention`; the only thing separating them is the number the host
     * computed, and the done agent is deliberately given the alphabetically
     * earlier id so that nothing but rank can produce this order.
     */
    @Test
    fun `blocked outranks done across workspaces, from the hosts rank alone`() {
        val sections = needsYou(
            listOf(
                input("a", workspace("aaa", terminals = listOf(agent("t1", "done", 199_999_989)))),
                input("a", workspace("zzz", terminals = listOf(agent("t2", "blocked", 99_999_939)))),
            )
        )
        assertEquals(listOf("zzz", "aaa"), sections.map { it.workspace.id })
    }

    /** The same rule inside one section, which is the other half of what rank buys. */
    @Test
    fun `blocked above finished inside one workspace`() {
        val section = needsYou(
            listOf(
                input(
                    "a",
                    workspace(
                        "w1",
                        terminals = listOf(
                            agent("done-1", "done", 199_999_989),
                            agent("blocked-1", "blocked", 99_999_939),
                        ),
                    ),
                )
            )
        ).single()
        assertEquals(listOf("blocked-1"), section.blocked.map { it.id })
        assertEquals(listOf("done-1"), section.finished.map { it.id })
    }

    /**
     * A workspace is as urgent as its MOST urgent agent. An average or a count
     * would let a worktree with six working agents outrank one with a single
     * agent stuck for an hour.
     */
    @Test
    fun `a workspace ranks by its lowest rank, not by how many agents it has`() {
        val sections = needsYou(
            listOf(
                input(
                    "a",
                    workspace(
                        "many",
                        terminals = listOf(
                            agent("m1", "done", 199_999_900),
                            agent("m2", "done", 199_999_901),
                            agent("m3", "done", 199_999_902),
                        ),
                    ),
                ),
                input(
                    "a",
                    // One agent, blocked for ten minutes.
                    workspace("stuck", terminals = listOf(agent("s1", "blocked", 99_999_399))),
                ),
            )
        )
        assertEquals(listOf("stuck", "many"), sections.map { it.workspace.id })
    }

    /** Oldest first inside a tier, which is what the host's subtraction encodes. */
    @Test
    fun `the agent stuck longest sorts first inside a section`() {
        val section = needsYou(
            listOf(
                input(
                    "a",
                    workspace(
                        "w1",
                        terminals = listOf(
                            // Blocked one minute.
                            agent("recent", "blocked", 99_999_939),
                            // Blocked ten minutes: a SMALLER number.
                            agent("ancient", "blocked", 99_999_399),
                        ),
                    ),
                )
            )
        ).single()
        assertEquals(listOf("ancient", "recent"), section.blocked.map { it.id })
    }

    /**
     * A daemon too old to send `rank` must not be able to outrank a known
     * blocked agent. `Terminal.sortRank` answers `Long.MAX_VALUE`, and this
     * pins that the front door respects it.
     */
    @Test
    fun `an agent with no rank sorts last rather than first`() {
        val section = needsYou(
            listOf(
                input(
                    "a",
                    workspace(
                        "w1",
                        terminals = listOf(
                            agent("unranked", "blocked", null),
                            agent("ranked", "blocked", 99_999_939),
                        ),
                    ),
                )
            )
        ).single()
        assertEquals(listOf("ranked", "unranked"), section.blocked.map { it.id })
    }

    /**
     * A finished agent goes above an unread diff. The perishable thing above
     * the durable one: a finished turn expires the moment somebody reads it, a
     * diff was true before the app was opened and stays true until it is.
     */
    @Test
    fun `a finished agent outranks an unread diff`() {
        val sections = needsYou(
            listOf(
                input("a", workspace("diff"), InboxRow("diff", true, 200, 40)),
                input("a", workspace("answer", terminals = listOf(agent("t", "done", 199_999_989)))),
            )
        )
        assertEquals(listOf("answer", "diff"), sections.map { it.workspace.id })
    }

    /** The second tier keeps the order the fleet arrived in, with no rank invented for it. */
    @Test
    fun `unread diffs keep fleet order`() {
        val sections = needsYou(
            listOf(
                input("b", workspace("second"), InboxRow("second", true, 1, 1)),
                input("a", workspace("first"), InboxRow("first", true, 900, 900)),
            )
        )
        assertEquals(listOf("second", "first"), sections.map { it.workspace.id })
    }

    // ---- across runners ----

    /**
     * The whole point of the merge. Runner `a` is listed first and its agent
     * merely finished; runner `b`'s agent is blocked, and it goes on top.
     * Nothing about which machine a pane is on is allowed to affect where it
     * sorts.
     */
    @Test
    fun `a blocked agent on the second runner outranks a finished one on the first`() {
        val sections = needsYou(
            listOf(
                input("a", workspace("w-a", terminals = listOf(agent("t-a", "done", 199_999_989)))),
                input("b", workspace("w-b", terminals = listOf(agent("t-b", "blocked", 99_999_939)))),
            )
        )
        assertEquals(listOf("b", "a"), sections.map { it.hostId })
    }

    /**
     * Ranks from two daemons are directly comparable, because both terms of
     * `feed::rank` — the tier and the state's AGE in seconds — are quantities
     * neither machine's wall clock enters. Two agents blocked for the same
     * length of time on two runners produce the identical integer, which is
     * exactly the collision the tiebreak exists for.
     */
    @Test
    fun `identical ranks on two runners break deterministically and not by input order`() {
        val a = input("alpha", workspace("w", terminals = listOf(agent("t", "blocked", 99_999_939))))
        val b = input("beta", workspace("w", terminals = listOf(agent("t", "blocked", 99_999_939))))

        assertEquals(listOf("alpha", "beta"), needsYou(listOf(a, b)).map { it.hostId })
        assertEquals(listOf("alpha", "beta"), needsYou(listOf(b, a)).map { it.hostId })
    }

    /**
     * Workspace ids are minted per daemon, so two runners can hold the same one.
     * `BackstackTest` pins that the routes stay apart; this pins that the front
     * door's own identity does too, because that string is what a `LazyColumn`
     * keys its items on and a collision there would draw one section for two
     * worktrees.
     */
    @Test
    fun `two runners sharing a workspace id are two sections with two keys`() {
        val sections = needsYou(
            listOf(
                input("a", workspace("shared", terminals = listOf(agent("t", "blocked", 99_999_939)))),
                input("b", workspace("shared", terminals = listOf(agent("t", "blocked", 99_999_938)))),
            )
        )
        assertEquals(2, sections.size)
        assertEquals(setOf("a/shared", "b/shared"), sections.map { it.key }.toSet())
    }

    /** Every section carries its runner's name, for the header that shows it. */
    @Test
    fun `a section carries the runner it came from`() {
        val section = needsYou(
            listOf(
                input(
                    "host-1",
                    workspace("w", terminals = listOf(agent("t", "blocked", 99_999_939))),
                    label = "studio",
                )
            )
        ).single()
        assertEquals("host-1", section.hostId)
        assertEquals("studio", section.hostLabel)
    }

    // ---- the diff row ----

    /**
     * Nil counts and zero counts are different states and get opposite answers.
     * "Not told yet" draws the row without numbers; "told, and it is clean"
     * draws nothing.
     */
    @Test
    fun `the changes row is present while unanswered and absent once called clean`() {
        fun showsChanges(counts: InboxRow?) = needsYou(
            listOf(
                input(
                    "a",
                    workspace("w", terminals = listOf(agent("t", "blocked", 99_999_939))),
                    counts,
                )
            )
        ).single().showsChanges

        assertTrue("no answer yet is not an answer of zero", showsChanges(null))
        assertFalse(showsChanges(InboxRow("w", changedSinceReviewed = true)))
        assertTrue(showsChanges(InboxRow("w", changedSinceReviewed = false, insertions = 1)))
    }

    /** Ordinals come with the section, computed once rather than once per row. */
    @Test
    fun `two panes with the same name are numbered`() {
        val section = needsYou(
            listOf(
                input(
                    "a",
                    workspace(
                        "w",
                        terminals = listOf(
                            agent("t1", "blocked", 99_999_939, preset = "claude"),
                            agent("t2", "blocked", 99_999_938, preset = "claude"),
                        ),
                    ),
                )
            )
        ).single()
        assertEquals(1, section.ordinals["t1"])
        assertEquals(2, section.ordinals["t2"])
    }

    // ---- the sentences a truncated group puts under itself ----

    @Test
    fun `blocked overflow counts in words`() {
        assertEquals("1 more agent needs you", blockedOverflow(1))
        assertEquals("4 more agents need you", blockedOverflow(4))
    }

    /**
     * A failure must never be swept into "finished", least of all here — the
     * only place on this screen where agents are counted instead of shown.
     */
    @Test
    fun `finished overflow says failed when any of the hidden ones did`() {
        val ok = agent("a", "done", 199_999_989)
        val died = agent("b", "done", 199_999_988).copy(turnFailed = true)

        assertEquals("1 more agent finished", finishedOverflow(listOf(ok)))
        assertEquals("2 more agents finished", finishedOverflow(listOf(ok, ok)))
        assertEquals("1 more agent failed", finishedOverflow(listOf(died)))
        assertEquals("2 more agents failed", finishedOverflow(listOf(died, died)))
        assertEquals("3 more agents finished, 1 failed", finishedOverflow(listOf(ok, ok, died)))
    }

    /** A `done` agent whose turn did NOT fail is not counted as one that did. */
    @Test
    fun `an absent turnFailed is not a failure`() {
        assertNull(agent("a", "done", 1).turnFailed)
        assertEquals("1 more agent finished", finishedOverflow(listOf(agent("a", "done", 1))))
    }

    // ---- fixtures ----

    private fun input(
        hostId: String,
        workspace: Workspace,
        counts: InboxRow? = null,
        label: String = hostId,
    ) = NeedsYouInput(hostId, label, workspace, counts)

    private fun workspace(
        id: String,
        state: String = "",
        terminals: List<Terminal> = emptyList(),
    ) = Workspace(id = id, task = id, branch = "feat/$id", state = state, terminals = terminals)

    private fun agent(
        id: String,
        activity: String,
        rank: Long?,
        preset: String = "claude",
    ) = Terminal(
        id = id,
        preset = preset,
        state = "running",
        activity = activity,
        rank = rank,
    )
}
