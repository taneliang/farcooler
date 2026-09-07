package com.farcooler.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The tunnel names its failures with a machine word, and this app owns the
 * sentence for each one.
 *
 * `SshError::Tunnel` renders as `cannot open the tunnel: <word>` and the word is
 * `farcooler_tailcat::TunnelError::code`. Every phrase [Connection.Failure.of]
 * used to match — "cannot reach", "did not answer", "no SSH key" — misses that
 * sentence completely, so all four tunnel failures fell to
 * [Connection.Failure.OTHER], which is the one kind that prints the core's own
 * text on screen. A device removed from a runner's allowlist read
 *
 *     cannot open the tunnel: no_answer
 *
 * off its own phone, in the one situation where a clear sentence matters most:
 * `no_answer` is specifically how a revoked device presents, because tailcat
 * ignores a client it does not recognize silently and the device gets a timeout
 * rather than a refusal.
 *
 * **The messages below are transcribed from Rust, not composed from
 * [Connection.Failure.TUNNEL_MARKER].** Building them out of the constant would
 * make this test agree with itself while the app disagreed with the core: a
 * reword of `SshError::Tunnel`'s `Display` would move the marker and the test
 * would follow it. Written out, a reword breaks this test, which is what a
 * cross-language string contract needs. `crates/client/src/ssh.rs`'s
 * `the_tunnel_message_carries_the_word_the_apps_read` holds the other end.
 *
 * They are also the WHOLE message and not a fragment. `SessionError::Ssh` is
 * `#[error(transparent)]` and `farcooler_client_connect` pushes `e.to_string()`,
 * so this is byte for byte what arrives in `Exception.message` on this side.
 */
class TunnelWordTest {

    /** Exactly what `SshError::Tunnel { code: "no_answer" }` renders as. */
    private val noAnswer = "cannot open the tunnel: no_answer"
    private val derp = "cannot open the tunnel: derp"
    private val noTailcat = "cannot open the tunnel: no_tailcat"
    private val io = "cannot open the tunnel: io"

    @Test
    fun theMarkerIsSpelledTheWayRustSpellsIt() {
        assertEquals(
            "crates/client/src/ssh.rs's SshError::Tunnel no longer prints this",
            "cannot open the tunnel: ",
            Connection.Failure.TUNNEL_MARKER,
        )
    }

    @Test
    fun eachStableWordNamesItsOwnFailure() {
        assertEquals(Connection.Failure.TUNNEL_NO_ANSWER, Connection.Failure.of(noAnswer))
        assertEquals(Connection.Failure.TUNNEL_RENDEZVOUS, Connection.Failure.of(derp))
        assertEquals(
            Connection.Failure.TUNNEL_NOT_IN_THIS_BUILD,
            Connection.Failure.of(noTailcat),
        )
        assertEquals(Connection.Failure.TUNNEL_UNSPECIFIED, Connection.Failure.of(io))
    }

    /**
     * The defect this file exists for, stated as itself.
     *
     * [Connection.Failure.OTHER] is the only kind `FleetScreen` puts the core's
     * raw text under, so a tunnel failure landing there is the machine word on a
     * screen. None of the four may be it.
     */
    @Test
    fun noTunnelFailureFallsToTheKindThatPrintsRawText() {
        for (message in listOf(noAnswer, derp, noTailcat, io)) {
            assertNotEquals(
                "$message would put its own word on screen",
                Connection.Failure.OTHER,
                Connection.Failure.of(message),
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

    /**
     * A fifth word added in Rust reaches a screen as a sentence somebody wrote.
     *
     * Not [Connection.Failure.OTHER]: that arm prints the message, and the
     * message is the word. The generic tunnel sentence claims nothing about
     * which failure this was, which is the correct thing to say about a word
     * this build has never seen.
     */
    @Test
    fun aWordThisBuildHasNeverSeenIsStillATunnelFailure() {
        assertEquals(
            Connection.Failure.TUNNEL_UNSPECIFIED,
            Connection.Failure.of("cannot open the tunnel: quic_refused"),
        )
    }

    /**
     * Wrapping the error in more context does not stop it matching, and the
     * context does not become part of the word.
     *
     * The same property every phrase in [Connection.Failure.of] has, and it
     * matters more here: a word read to the end of the string would make
     * `no_answer (attempt 3)` an unrecognized word and downgrade a revoked
     * device to the generic sentence.
     */
    @Test
    fun theWordIsFoundInContextAndEndsAtTheFirstSpace() {
        assertEquals(
            Connection.Failure.TUNNEL_NO_ANSWER,
            Connection.Failure.of("dialing studio: cannot open the tunnel: no_answer (attempt 3)"),
        )
    }

    /**
     * The eight phrases that were already here still classify, and the tunnel
     * word being read first did not steal any of them.
     *
     * Every string is one a runner really produces — transcribed from
     * `crates/client/src/ssh.rs`, `session.rs`, `Connection.declineHostKey`,
     * `Connection.giveUp` and `Identity` — rather than the shortest string that
     * satisfies the substring being matched. A table tested against strings
     * invented to please it is a table that proves nothing.
     */
    @Test
    fun theOrdinaryFailuresAreUnchanged() {
        assertEquals(
            Connection.Failure.KEY_REJECTED,
            Connection.Failure.of(
                "me@10.0.0.4 rejected this key. " +
                    "Add its public key to ~/.ssh/authorized_keys there."
            ),
        )
        assertEquals(
            Connection.Failure.HOST_KEY_CHANGED,
            Connection.Failure.of(
                "the host key for 10.0.0.4 is not the one Far Cooler has recorded.\n" +
                    "Expected SHA256:aaa\nGot      SHA256:bbb\n" +
                    "This is either a changed server or an interception. " +
                    "Far Cooler will not connect."
            ),
        )
        assertEquals(
            Connection.Failure.UNREACHABLE,
            Connection.Failure.of(
                "cannot reach 10.0.0.4:2222: Connection refused (os error 61)"
            ),
        )
        assertEquals(
            Connection.Failure.DAEMON_MISSING,
            Connection.Failure.of(
                "connected, but `farcoolerd --stdio` did not answer. " +
                    "Is Far Cooler installed on that runner?"
            ),
        )
        assertEquals(
            Connection.Failure.NO_IDENTITY,
            Connection.Failure.of(
                "This device has no SSH key and one could not be generated."
            ),
        )
        assertEquals(
            Connection.Failure.NO_NODE_KEY,
            Connection.Failure.of(Connection.NO_NODE_KEY_SENTENCE),
        )
        assertEquals(
            Connection.Failure.KEY_NOT_TRUSTED,
            Connection.Failure.of(
                "The key Studio presented has not been trusted on this device. " +
                    "Far Cooler won’t connect until it is."
            ),
        )
        assertEquals(
            Connection.Failure.STOPPED,
            Connection.Failure.of(
                "Stopped waiting for Studio. It may be asleep or off the network."
            ),
        )
    }

    /**
     * The one tunnel failure that is NOT a stable word, and stays where it was.
     *
     * `SshError::TunnelPortClosed` — the tunnel reached the runner and nothing
     * was listening for SSH — deliberately carries a written sentence rather
     * than a code, with the OS error kept as `#[source]` for logs and never
     * interpolated. So it lands on [Connection.Failure.OTHER] and `FleetScreen`
     * puts that sentence in the detail box, which is English somebody wrote and
     * safe to show. It is recorded here so a later reword of it is a decision
     * rather than an accident.
     */
    @Test
    fun aTunnelThatReachedTheRunnerIsNotOneOfTheFourWords() {
        assertEquals(
            Connection.Failure.OTHER,
            Connection.Failure.of(
                "the tunnel reached the runner, but nothing is listening for SSH there"
            ),
        )
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
