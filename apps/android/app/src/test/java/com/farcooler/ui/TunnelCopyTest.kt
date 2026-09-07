package com.farcooler.ui

import com.farcooler.data.Reach
import com.farcooler.data.Runner
import com.farcooler.net.Connection
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What a person reads when the tunnel does not open.
 *
 * The core sends a stable machine word and this app owns the sentence — the
 * standing rule, and the one this screen broke. All four tunnel failures used to
 * classify as [Connection.Failure.OTHER], which is the single kind whose detail
 * is the core's own text and whose `DetailBox` prints the raw message
 * underneath, so a revoked device read `cannot open the tunnel: no_answer`.
 *
 * [failureHeadline] and [failureDetail] take a [Runner] rather than a
 * `Connection` so this test can call them at all: a `Connection` holds a
 * `ClientCore` and a coroutine scope and cannot be built off a device.
 *
 * `crates/cli/src/runner_pipe.rs`'s `sentence` says the same four things to a
 * terminal and `RunnerTrouble` in `apps/shared/AgentKit` says them on the Apple
 * apps. Three readers of one dialect; reword one and the others are where to
 * look.
 */
class TunnelCopyTest {

    /**
     * A tunneled runner, which is the only kind that can produce these failures.
     *
     * It has no address at all — that is what `Reach.Tailcat` means — so a
     * sentence about it can only be built out of the label the granting device
     * sent. `Runner.named` is what does that.
     */
    private val tunneled = Runner(
        id = "r1",
        label = "Studio",
        reach = Reach.Tailcat("tok"),
        user = "me",
    )

    /** The four kinds, and the word each one was named by. */
    private val tunnelKinds = listOf(
        Connection.Failure.TUNNEL_NO_ANSWER,
        Connection.Failure.TUNNEL_RENDEZVOUS,
        Connection.Failure.TUNNEL_NOT_IN_THIS_BUILD,
        Connection.Failure.TUNNEL_UNSPECIFIED,
    )

    /**
     * The words that may never appear as words.
     *
     * Compared token by token rather than by `contains`, because `io` and `derp`
     * are short enough to sit inside ordinary English and a substring check
     * would either miss a real leak or fail on a sentence that was fine.
     */
    private val machineWords = setOf("no_answer", "derp", "no_tailcat", "io")

    private fun words(text: String): List<String> =
        text.lowercase().split(Regex("[^a-z0-9_]+")).filter { it.isNotEmpty() }

    private fun assertNoMachineWord(where: String, text: String) {
        assertFalse(
            "$where printed the marker the core wraps the word in: $text",
            text.contains(Connection.Failure.TUNNEL_MARKER),
        )
        val leaked = words(text).filter { it in machineWords }
        assertTrue("$where printed the machine word $leaked: $text", leaked.isEmpty())
    }

    /**
     * The defect, on the surface it appeared on.
     *
     * Both lines, for all four words, and the message they are given is the real
     * one — so a table that fell back to `message` for any of them would put
     * `cannot open the tunnel: <word>` straight into this assertion.
     */
    @Test
    fun noTunnelSentenceContainsTheWordItWasNamedBy() {
        for (kind in tunnelKinds) {
            val message = "cannot open the tunnel: ${kind.tunnelWord}"
            assertNoMachineWord("$kind headline", failureHeadline(kind, tunneled))
            assertNoMachineWord("$kind detail", failureDetail(kind, tunneled, message))
        }
    }

    /** Every one of them says something, and none of them says nothing. */
    @Test
    fun everyTunnelFailureHasBothLines() {
        for (kind in tunnelKinds) {
            val message = "cannot open the tunnel: ${kind.tunnelWord}"
            assertTrue("$kind", failureHeadline(kind, tunneled).isNotBlank())
            assertTrue("$kind", failureDetail(kind, tunneled, message).isNotBlank())
        }
    }

    /**
     * A revoked device and a sleeping runner are indistinguishable from here —
     * tailcat ignores a client it does not recognize silently, so both are a
     * timeout — and the sentence names both. Naming only one would send half the
     * people who read it to the wrong place.
     */
    @Test
    fun theTimeoutNamesBothThingsItCouldBe() {
        val detail = failureDetail(
            Connection.Failure.TUNNEL_NO_ANSWER,
            tunneled,
            "cannot open the tunnel: no_answer",
        )
        assertTrue(detail, detail.contains("asleep"))
        assertTrue(detail, detail.contains("revoked"))
    }

    /**
     * A timeout through the tunnel is the same fact about the same runner as a
     * runner nobody could reach, so it is headlined the same way and by name.
     * `Runner.named` is what makes that a label rather than the empty address a
     * tunneled runner has.
     */
    @Test
    fun theTimeoutNamesTheRunner() {
        assertEquals(
            "Can’t reach Studio",
            failureHeadline(Connection.Failure.TUNNEL_NO_ANSWER, tunneled),
        )
    }

    /**
     * The rendezvous is the one that must NOT name the runner. What could not be
     * reached is the service that introduces this device to it, and blaming the
     * runner would send somebody to go and wake a machine that was awake the
     * whole time — so the sentence points at this device's own network instead.
     */
    @Test
    fun theRendezvousBlamesThisDeviceAndNotTheRunner() {
        val headline = failureHeadline(Connection.Failure.TUNNEL_RENDEZVOUS, tunneled)
        assertFalse(headline, headline.contains("Studio"))
        val detail = failureDetail(
            Connection.Failure.TUNNEL_RENDEZVOUS,
            tunneled,
            "cannot open the tunnel: derp",
        )
        assertFalse(detail, detail.contains("Studio"))
        assertTrue(detail, detail.contains("this device’s own network"))
    }

    /**
     * A build with no tunnel in it blames neither the runner nor the network:
     * nothing about either one changes the answer.
     */
    @Test
    fun aBuildWithNoTunnelBlamesTheBuild() {
        val detail = failureDetail(
            Connection.Failure.TUNNEL_NOT_IN_THIS_BUILD,
            tunneled,
            "cannot open the tunnel: no_tailcat",
        )
        assertTrue(detail, detail.contains("build of Far Cooler"))
        assertFalse(detail, detail.contains("Studio"))
    }

    /**
     * `io` is deliberately generic upstream — a malformed token, a dead sshd
     * whose errno differs by platform, and `EMFILE` all wear it — so the sentence
     * must claim nothing about which. A guess here sends somebody to fix
     * something that was never the problem.
     */
    @Test
    fun theGenericWordClaimsNothingAboutTheCause() {
        val detail = failureDetail(
            Connection.Failure.TUNNEL_UNSPECIFIED,
            tunneled,
            "cannot open the tunnel: io",
        )
        assertEquals("The tunnel couldn’t be opened.", detail)
    }

    /**
     * The app's apostrophes are curly, everywhere, which is the convention the
     * ~122 existing ones in this app already follow. A straight one is a
     * different glyph in the same sentence and reads as a typo next to them.
     */
    @Test
    fun everyTunnelSentenceUsesCurlyApostrophes() {
        for (kind in tunnelKinds) {
            val message = "cannot open the tunnel: ${kind.tunnelWord}"
            for (line in listOf(
                failureHeadline(kind, tunneled),
                failureDetail(kind, tunneled, message),
            )) {
                assertFalse("$kind used a straight apostrophe: $line", line.contains('\''))
            }
        }
    }

    /**
     * The whole path, end to end: the string the core really hands up, through
     * the classifier, to the two lines a person reads.
     *
     * The tests above name the kind directly, which would go on passing if
     * [Connection.Failure.of] stopped producing it. This one starts where the
     * app starts.
     */
    @Test
    fun aRevokedDeviceReadsASentenceAndNotAWord() {
        val message = "cannot open the tunnel: no_answer"
        val kind = Connection.Failure.of(message)
        assertEquals(Connection.Failure.TUNNEL_NO_ANSWER, kind)
        assertEquals("Can’t reach Studio", failureHeadline(kind, tunneled))
        assertEquals(
            "It didn’t answer. The runner may be asleep, or this device’s access " +
                "to it may have been revoked.",
            failureDetail(kind, tunneled, message),
        )
    }

    /**
     * The kinds that still pass the core's text through do so unchanged.
     *
     * Five of them, all carrying sentences somebody wrote — the changed host
     * key's names the two fingerprints being compared and must not be
     * paraphrased. Guarded here because the `else` arm that used to do this was
     * replaced by five named arms, and a mistake in that swap would be silent.
     */
    @Test
    fun theWrittenSentencesStillReachTheScreen() {
        for (kind in listOf(
            Connection.Failure.HOST_KEY_CHANGED,
            Connection.Failure.NO_IDENTITY,
            Connection.Failure.NO_NODE_KEY,
            Connection.Failure.KEY_NOT_TRUSTED,
            Connection.Failure.STOPPED,
        )) {
            assertEquals("$kind", "a sentence", failureDetail(kind, tunneled, "a sentence"))
        }
    }
}
