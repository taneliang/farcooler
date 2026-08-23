package com.farcooler.model

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Every key the client core puts on a fleet, decoded.
 *
 * The fleet payload is built in one place — `crates/client/src/session.rs`, the
 * `json!` block at lines 260-374 — and this app decodes it with
 * `ignoreUnknownKeys = true` (`net/Connection.kt:192`). Those two facts together
 * are why seventeen fields could arrive on every poll for two months and be
 * dropped in silence: a key this side does not declare is not a warning, not a
 * log line and not a crash. It is a row that says less than the daemon knows.
 *
 * So the spelling is what is tested. The JSON below is transcribed key-for-key
 * from that `json!` block, and every assertion is that a value put on the wire
 * came out the other side under the name this app reads it by. A field renamed
 * on either end fails here rather than going quiet on a phone.
 *
 * Values are deliberately non-default — `false` where the default is `false` is
 * a test that passes when the decode does nothing at all.
 */
class FleetDecodeTest {
    /** The same configuration `Connection` decodes a fleet with. */
    private val json = Json { ignoreUnknownKeys = true }

    private val payload = """
        {
          "runtime_healthy": true,
          "live_panes": 4,
          "workspaces": [
            {
              "id": "8f14e45f-ce5b-4a5e-9c2b-000000000001",
              "short": "8f14e4",
              "repository": "1c383cd3-0b0f-4a63-b8a1-000000000002",
              "task": "Widen the model",
              "branch": "widen-the-model",
              "worktree": "/Users/e/src/overnight-widen",
              "state": "worktree_missing",
              "isMainCheckout": true,
              "terminals": [
                {
                  "id": "aab3238922bcc25a6f606eb525ffdc56",
                  "short": "aab323",
                  "title": "Fix the parser",
                  "preset": "claude",
                  "state": "exited",
                  "activity": "done",
                  "activitySince": 1755900000000,
                  "exitCode": 101,
                  "exitSignal": 9,
                  "turnStartedAt": 1755899000000,
                  "blockedQuestion": "Run `rm -rf build`?",
                  "feed": ["Reading watch.rs.", "Rewrote the poller.", "Ran the suite."],
                  "said": "Rewrote the poller so the filesystem says when something moved.",
                  "subagents": ["explore", "plan"],
                  "glyph": "✓",
                  "headline": "Done · claude",
                  "line": "3/7 · Designing test matrix",
                  "rank": 199999940,
                  "planDone": 3,
                  "planTotal": 7,
                  "turnFailed": true,
                  "epoch": 12,
                  "paneMode": "changes",
                  "chatCapable": true,
                  "agentSessionId": "01J8Z2",
                  "agentMode": "plan",
                  "availableAgentModes": ["plan", "edit"]
                }
              ]
            }
          ]
        }
    """.trimIndent()

    private val terminal: Terminal
        get() = json.decodeFromString(Fleet.serializer(), payload)
            .workspaces.first().terminals.first()

    @Test
    fun everyTerminalFieldOnTheWireLandsOnTheModel() {
        val t = terminal
        assertEquals("aab3238922bcc25a6f606eb525ffdc56", t.id)
        assertEquals("aab323", t.short)
        assertEquals("Fix the parser", t.title)
        assertEquals("claude", t.preset)
        assertEquals("exited", t.state)
        assertEquals("done", t.activity)
        assertEquals(1755900000000.0, t.activitySince!!, 0.0)
        assertEquals(101, t.exitCode)
        assertEquals(9, t.exitSignal)
        assertEquals(1755899000000.0, t.turnStartedAt!!, 0.0)
        assertEquals("Run `rm -rf build`?", t.blockedQuestion)
        assertEquals(listOf("Reading watch.rs.", "Rewrote the poller.", "Ran the suite."), t.feed)
        assertTrue(t.said!!.startsWith("Rewrote the poller"))
        assertEquals(listOf("explore", "plan"), t.subagents)
        assertEquals("✓", t.glyph)
        assertEquals("Done · claude", t.headline)
        assertEquals("3/7 · Designing test matrix", t.line)
        assertEquals(199_999_940L, t.rank)
        assertEquals(3, t.planDone)
        assertEquals(7, t.planTotal)
        assertEquals(true, t.turnFailed)
        assertEquals(12, t.epoch)
        assertEquals("changes", t.paneMode)
        assertEquals(true, t.chatCapable)
        assertEquals("01J8Z2", t.agentSessionId)
        assertEquals("plan", t.agentMode)
        assertEquals(listOf("plan", "edit"), t.availableAgentModes)
    }

    @Test
    fun everyWorkspaceFieldOnTheWireLandsOnTheModel() {
        val w = json.decodeFromString(Fleet.serializer(), payload).workspaces.first()
        assertEquals("1c383cd3-0b0f-4a63-b8a1-000000000002", w.repository)
        assertEquals("Widen the model", w.task)
        assertEquals("widen-the-model", w.branch)
        assertEquals("/Users/e/src/overnight-widen", w.worktree)
        // `isMainCheckout`, in camelCase, which is what the CLIENT CORE sends.
        // The CLI spells the same flag `is_main_checkout` and the Mac's model
        // matches the CLI; iOS took the Mac's property name onto this payload
        // and decodes nothing.
        assertTrue(w.isMainCheckout)
        assertTrue(w.worktreeMissing)
        assertFalse(w.isHidden)
    }

    @Test
    fun aFleetFromADaemonThatSendsNoneOfThemStillDecodes() {
        // The whole reason every added field is nullable. A runner that has not
        // been reinstalled since these landed must cost one row its detail, not
        // the entire fleet its screen.
        val old = """
            {"runtime_healthy":true,"live_panes":1,"workspaces":[
              {"id":"w","short":"w","task":"t","branch":"b","state":"active",
               "terminals":[{"id":"t1","short":"t1","title":"","preset":"zsh",
                             "state":"running","epoch":1}]}]}
        """.trimIndent()
        val t = json.decodeFromString(Fleet.serializer(), old)
            .workspaces.first().terminals.first()
        assertNull(t.rank)
        assertNull(t.feed)
        assertNull(t.said)
        assertNull(t.line)
        assertNull(t.activitySince)
        assertNull(t.exitCode)
        assertNull(t.turnFailed)
        assertFalse(t.runDidFail)
        assertEquals(Long.MAX_VALUE, t.sortRank)
        // And nothing about the row it draws is a claim: absent is not zero and
        // not false-as-an-answer.
        assertEquals(emptyList<String>(), t.recentSteps)
        assertNull(t.lastSaid)
        assertEquals("", t.signalLine)
    }

    /**
     * `host.health`, transcribed key for key.
     *
     * Shaped in `crates/client/src/ffi.rs` rather than in `session.rs` — the
     * only reason it is worth saying is that this file's header points at
     * `session.rs` for everything else, and a reader checking the spelling has
     * to be sent to the right producer. This is the same discipline for the same
     * reason: `ignoreUnknownKeys` means a key spelled wrong here is a runner
     * that reports itself healthy forever, silently, which for THIS payload
     * would be the app suppressing the daemon's own account of what is wrong
     * with it.
     */
    @Test
    fun aRunnersHealthDecodesEveryKeyTheFfiEmits() {
        val payload = """
            {
              "platform": "linux",
              "daemonVersion": "0.1.0+9f2c1ab",
              "protocolVersion": 1,
              "healthy": false,
              "reasons": ["tmux server is not reachable", "review cache is rebuilding"],
              "livePanes": 6
            }
        """.trimIndent()

        val health = json.decodeFromString(HostHealth.serializer(), payload)
        assertEquals("linux", health.platform)
        assertEquals("0.1.0+9f2c1ab", health.daemonVersion)
        assertEquals(1, health.protocolVersion)
        assertFalse(health.healthy)
        // Every one of them, in order, unsummarized. This is the whole field.
        assertEquals(
            listOf("tmux server is not reachable", "review cache is rebuilding"),
            health.reasons,
        )
        assertEquals(6, health.livePanes)
    }

    /**
     * A runner too old to answer costs a row, not the decode — and the direction
     * a missing `healthy` falls is the one that says nothing.
     */
    @Test
    fun anEmptyHealthReadsAsWellRatherThanAsBroken() {
        val health = json.decodeFromString(HostHealth.serializer(), "{}")
        // Not `false`. A runner that never said is not a runner that said no,
        // and drawing "Degraded" over silence is the fleet footer's old mistake
        // in a new place.
        assertTrue(health.healthy)
        assertEquals(emptyList<String>(), health.reasons)
        assertEquals(0, health.livePanes)
        assertEquals("", health.platform)
    }

    @Test
    fun aKeyThisBuildHasNeverHeardOfIsNotAnError() {
        // The setting that hid seventeen fields is also what lets an older app
        // survive a newer runner, which is why it stays. It is tested here so
        // the trade is stated somewhere rather than assumed.
        val ahead = """
            {"runtime_healthy":true,"live_panes":1,"workspaces":[
              {"id":"w","short":"w","task":"t","branch":"b","state":"active",
               "somethingNewer":{"a":1},
               "terminals":[{"id":"t1","short":"t1","title":"","preset":"zsh",
                             "state":"running","epoch":1,"tokenBudget":42}]}]}
        """.trimIndent()
        val fleet = json.decodeFromString(Fleet.serializer(), ahead)
        assertEquals("t1", fleet.workspaces.first().terminals.first().id)
    }
}
