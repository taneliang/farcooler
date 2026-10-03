package com.farcooler.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A failed connect is classified by the core's word, and this app owns the
 * sentence for each one (ov-127).
 *
 * The connect line carries `trouble` (`SessionError::word` in
 * `crates/client`) and, for a tunnel failure, the tunnel's own word as
 * `tunnel` (`farcooler_tailcat::TunnelError::code`). This used to match
 * phrases out of Rust's `Display` strings — "cannot reach", "rejected this
 * key", and `cannot open the tunnel: <word>` — so a reword in `ssh.rs` changed
 * which button a person was offered with every test on both sides still
 * green. `crates/client`'s `every_ssh_error_has_a_word_of_its_own` and
 * `a_failed_connect_names_its_trouble_by_word` hold the Rust end.
 */
class TunnelWordTest {

    /**
     * Every `trouble` word the core can send. Copied by hand from the arms of
     * `SshError::word` and `SessionError::word`; the Rust test stops two
     * failures sharing a word, this stops a word this table expects going
     * unread.
     */
    private val everyCoreWord = listOf(
        "unreachable", "handshake_failed", "key_rejected", "bad_key", "host_key_changed",
        "host_key_unknown", "exec_failed", "tunnel", "tunnel_port_closed", "protocol",
        "wrong_result", "daemon_missing", "version_mismatch", "refused", "disconnected",
        "bad_config",
    )

    @Test
    fun eachStableWordNamesItsOwnFailure() {
        assertEquals(Connection.Failure.TUNNEL_NO_ANSWER, Connection.Failure.of("tunnel", "no_answer"))
        assertEquals(Connection.Failure.TUNNEL_RENDEZVOUS, Connection.Failure.of("tunnel", "derp"))
        assertEquals(
            Connection.Failure.TUNNEL_NOT_IN_THIS_BUILD,
            Connection.Failure.of("tunnel", "no_tailcat"),
        )
        assertEquals(Connection.Failure.TUNNEL_UNSPECIFIED, Connection.Failure.of("tunnel", "io"))
    }

    /**
     * The defect the tunnel words were added for: [Connection.Failure.OTHER]
     * is the only kind `FleetScreen` puts the core's raw text under, so a
     * tunnel failure landing there is the machine word on a screen.
     */
    @Test
    fun noTunnelFailureFallsToTheKindThatPrintsRawText() {
        for (word in listOf("no_answer", "derp", "no_tailcat", "io", "quic_refused", null)) {
            assertNotEquals(
                "$word would put the core's words on screen",
                Connection.Failure.OTHER,
                Connection.Failure.of("tunnel", word),
            )
        }
    }

    /**
     * The four words Rust sends, and no fifth invented here.
     *
     * A word this app carries that `TunnelError::code` does not send is a
     * sentence nothing will ever show; a word Rust sends that this app does not
     * carry lands on [Connection.Failure.TUNNEL_UNSPECIFIED] — safe, but wrong,
     * and this is where somebody finds out.
     */
    @Test
    fun theWordsAreTheOnesTunnelErrorCodeSends() {
        assertEquals(
            setOf("no_answer", "derp", "no_tailcat", "io"),
            Connection.Failure.entries.mapNotNull { it.tunnelWord }.toSet(),
        )
    }

    /** A fifth word added in Rust reaches a screen as a sentence somebody wrote. */
    @Test
    fun aWordThisBuildHasNeverSeenIsStillATunnelFailure() {
        assertEquals(
            Connection.Failure.TUNNEL_UNSPECIFIED,
            Connection.Failure.of("tunnel", "quic_refused"),
        )
    }

    /** The kinds a word decides, by the word and never by the prose. */
    @Test
    fun theOrdinaryFailuresAreReadByWord() {
        assertEquals(Connection.Failure.KEY_REJECTED, Connection.Failure.of("key_rejected"))
        assertEquals(Connection.Failure.HOST_KEY_CHANGED, Connection.Failure.of("host_key_changed"))
        assertEquals(Connection.Failure.UNREACHABLE, Connection.Failure.of("unreachable"))
        assertEquals(Connection.Failure.DAEMON_MISSING, Connection.Failure.of("daemon_missing"))
    }

    /**
     * Every other word, no word, and a word from a newer core are
     * [Connection.Failure.OTHER]. That includes `tunnel_port_closed` — the
     * tunnel reached the runner and nothing was listening for SSH — whose
     * written sentence `FleetScreen` shows in the detail box.
     */
    @Test
    fun aWordWithNoCaseIsUndiagnosed() {
        for (word in listOf("handshake_failed", "bad_key", "exec_failed", "tunnel_port_closed", "quic", null)) {
            assertEquals("$word", Connection.Failure.OTHER, Connection.Failure.of(word))
        }
    }

    /**
     * The four kinds this app raises itself are never a reading of the core's
     * words: they are set beside their sentence, and a core word that mapped
     * onto one would put a decision nobody made on a screen.
     */
    @Test
    fun noCoreWordIsAKindTheAppRaises() {
        val raised = setOf(
            Connection.Failure.NO_IDENTITY, Connection.Failure.NO_NODE_KEY,
            Connection.Failure.KEY_NOT_TRUSTED, Connection.Failure.STOPPED,
        )
        for (word in everyCoreWord) {
            assertTrue("$word", Connection.Failure.of(word) !in raised)
        }
    }

    /** The first-contact question is the word plus the fingerprint field, and nothing else. */
    @Test
    fun theHostKeyQuestionIsTheWordAndTheField() {
        assertEquals(
            "SHA256:abc",
            Connection.Failure.hostKeyQuestion("host_key_unknown", "SHA256:abc"),
        )
        assertNull(Connection.Failure.hostKeyQuestion("host_key_unknown", null))
        assertNull(Connection.Failure.hostKeyQuestion("host_key_unknown", ""))
        assertNull(Connection.Failure.hostKeyQuestion("host_key_changed", "SHA256:abc"))
    }

    /**
     * Whether a failure is chased on a schedule, read back from the table that
     * decides it.
     *
     * This moved out of `Connection.retryOrGiveUp` to be readable at all: a
     * `Connection` holds a `ClientCore` and a coroutine scope, so no JVM unit
     * test can build one, and the schedule was a `when` in a file CI compiles
     * and never runs.
     */
    @Test
    fun aBuildWithNoTunnelIsNeverChasedOnASchedule() {
        // A dial cannot put the Go library into an APK that shipped without
        // one, so a schedule here is a timeout every thirty seconds forever for
        // an answer that cannot change.
        assertEquals(
            Connection.Retry.NEVER,
            Connection.Failure.TUNNEL_NOT_IN_THIS_BUILD.retry,
        )
    }

    @Test
    fun theTransientTunnelFailuresAreWorthChasing() {
        // A sleeping runner wakes, a rendezvous comes back, and `io` covers
        // enough transient things to be worth one more dial.
        for (kind in listOf(
            Connection.Failure.TUNNEL_NO_ANSWER,
            Connection.Failure.TUNNEL_RENDEZVOUS,
            Connection.Failure.TUNNEL_UNSPECIFIED,
        )) {
            assertEquals("$kind", Connection.Retry.ON_THE_BACKOFF, kind.retry)
        }
    }

    /**
     * The schedule for every other kind, exactly as `retryOrGiveUp` had it
     * before the table existed. A move that changed one of these would have
     * changed behavior while looking like a refactor.
     */
    @Test
    fun theScheduleForEveryOtherKindIsUnchanged() {
        for (kind in listOf(
            Connection.Failure.KEY_REJECTED,
            Connection.Failure.HOST_KEY_CHANGED,
            Connection.Failure.NO_IDENTITY,
            Connection.Failure.NO_NODE_KEY,
            Connection.Failure.KEY_NOT_TRUSTED,
        )) {
            assertEquals("$kind", Connection.Retry.NEVER, kind.retry)
        }
        assertEquals(Connection.Retry.AFTER_A_WHILE, Connection.Failure.DAEMON_MISSING.retry)
        for (kind in listOf(
            Connection.Failure.UNREACHABLE,
            Connection.Failure.STOPPED,
            Connection.Failure.OTHER,
        )) {
            assertEquals("$kind", Connection.Retry.ON_THE_BACKOFF, kind.retry)
        }
    }

    /**
     * "Try again" is not offered a second time under the primary action for any
     * tunnel failure, because for all four it IS the primary action.
     */
    @Test
    fun noTunnelFailureOffersRetryingTwice() {
        for (kind in listOf(
            Connection.Failure.TUNNEL_NO_ANSWER,
            Connection.Failure.TUNNEL_RENDEZVOUS,
            Connection.Failure.TUNNEL_NOT_IN_THIS_BUILD,
            Connection.Failure.TUNNEL_UNSPECIFIED,
        )) {
            assertTrue("$kind", !kind.worthRetryingAsAlternative)
        }
    }
}
