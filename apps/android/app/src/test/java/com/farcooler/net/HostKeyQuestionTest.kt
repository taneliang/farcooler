package com.farcooler.net

import com.farcooler.data.Reach
import com.farcooler.data.Runner
import com.farcooler.ui.failureDetail
import com.farcooler.ui.failureHeadline
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The fingerprint question, the way out of it, and the way back.
 *
 * Every link in that circle is a string match or a table lookup, and not one of
 * them fails loudly on its own: a reworded sentence stops classifying, a
 * dropped answer stops being drawn, and neither shows up in a build. This suite
 * is what makes each of them go red.
 *
 * **It exists because the answer that got dropped was dropped for real.**
 * `RunnerStatusRow` offered "Trust it" and "Edit" and nothing else, so somebody
 * who could not vouch for a fingerprint could agree to it or force-quit the
 * app; [Connection.declineHostKey] was written, with a comment naming the
 * screen it should land on, and called from nowhere. `53c6c57` fixed the same
 * defect on the same row on iOS, under `HostKeyQuestionTests`, and this is that
 * suite's other half.
 */
class HostKeyQuestionTest {

    /** Any runner at all: these sentences are built from a name and nothing else. */
    private val runner = Runner(
        id = "r1",
        label = "Studio",
        reach = Reach.Direct("box.local", 22),
        user = "me",
    )

    // MARK: every answer survives

    @Test
    fun theQuestionOffersAllThreeAnswers() {
        assertEquals(
            listOf(
                HostKeyQuestion.Answer.TRUST_IT,
                HostKeyQuestion.Answer.NOT_NOW,
                HostKeyQuestion.Answer.EDIT,
            ),
            HostKeyQuestion.Answer.entries,
        )
    }

    /**
     * The one that was missing, named on its own so a diff that drops it names
     * itself rather than moving a count from three to two.
     */
    @Test
    fun backingOutIsOffered() {
        assertTrue(
            "a fingerprint nobody can vouch for needs an answer that is not yes",
            HostKeyQuestion.Answer.NOT_NOW in HostKeyQuestion.Answer.entries,
        )
    }

    /**
     * Sentence case, which is this screen's own convention and not iOS's.
     *
     * Spelled out rather than derived, for the reason `TunnelWordTest` spells
     * out the Rust messages: a test that builds the expectation the same way
     * the code does agrees with itself for any wording at all.
     */
    @Test
    fun theAnswersReadTheWayThisScreensOtherButtonsDo() {
        assertEquals(
            listOf("Trust it", "Not now", "Edit"),
            HostKeyQuestion.Answer.entries.map { it.label },
        )
    }

    /** Three buttons at one emphasis is three things to weigh; one of them answers. */
    @Test
    fun exactlyOneAnswerIsTheAnswer() {
        assertEquals(
            listOf(HostKeyQuestion.Answer.TRUST_IT),
            HostKeyQuestion.Answer.entries.filter { it.isTheAnswer },
        )
    }

    /** No two buttons may read the same, or one of them cannot be chosen on purpose. */
    @Test
    fun noTwoAnswersReadAlike() {
        val labels = HostKeyQuestion.Answer.entries.map { it.label }
        assertEquals(labels.size, labels.toSet().size)
    }

    // MARK: the circle

    /**
     * Backing out has to land somewhere the app can name. The sentence is
     * written in one file and matched in the same one, and this is what stops a
     * reword turning a decision somebody made into "Can't connect".
     */
    @Test
    fun decliningClassifiesAsKeyNotTrusted() {
        val said = Connection.Failure.Said.declined(runner.named)
        assertEquals(Connection.Failure.KEY_NOT_TRUSTED, Connection.Failure.of(said))
        assertEquals(HostKeyQuestion.declining, Connection.Failure.of(said))
    }

    /**
     * Not a fault, and it must not be dressed as one: no automatic dial, and no
     * "Try again" offered underneath for a question that is simply still open.
     */
    @Test
    fun decliningIsNotTreatedAsAFault() {
        assertEquals(Connection.Retry.NEVER, HostKeyQuestion.declining.retry)
        assertFalse(HostKeyQuestion.declining.worthRetryingAsAlternative)
    }

    /**
     * The sentence reaches the screen whole.
     *
     * [Connection.Failure.KEY_NOT_TRUSTED] is one of the five kinds whose
     * detail is [Connection.Phase.Failed.message] passed through, so a wording
     * that classifies correctly and then gets paraphrased on the way to a row
     * would still be wrong. The headline is the app's own and names the state
     * without shouting about it.
     */
    @Test
    fun whatDecliningPutsOnScreenIsTheSentenceItself() {
        val said = Connection.Failure.Said.declined(runner.named)
        assertEquals(said, failureDetail(Connection.Failure.KEY_NOT_TRUSTED, runner, said))
        assertEquals("Key not trusted", failureHeadline(Connection.Failure.KEY_NOT_TRUSTED, runner))
    }

    /** Whoever declined has to be able to tell which runner they declined. */
    @Test
    fun theSentenceNamesTheRunner() {
        assertTrue(Connection.Failure.Said.declined("box.local").contains("box.local"))
        assertTrue(Connection.Failure.Said.stoppedWaiting("box.local").contains("box.local"))
    }

    // MARK: the sentences this app writes

    /**
     * The two sentences `abandon` composes, read back through the classifier
     * that has to recognize them. A reword breaks exactly one of these, by
     * name.
     *
     * Only two, deliberately. Everywhere else this app writes its own failure
     * text it states the kind beside it — `NO_NODE_KEY_SENTENCE` and the
     * no-identity arm of `Connection.start` both pass one — so those words are
     * read by a person and by nothing else. These two are the ones whose
     * meaning is recovered from their own wording.
     */
    @Test
    fun bothSentencesTheAppWritesClassifyBack() {
        assertEquals(
            Connection.Failure.KEY_NOT_TRUSTED,
            Connection.Failure.of(Connection.Failure.Said.declined("box.local")),
        )
        assertEquals(
            Connection.Failure.STOPPED,
            Connection.Failure.of(Connection.Failure.Said.stoppedWaiting("box.local")),
        )
    }

    /**
     * They are told APART, not merely recognized. A phrase generic enough to
     * match both would pass the test above and still send somebody to the wrong
     * button — "Show the key again" for a runner nobody ever asked about.
     */
    @Test
    fun theTwoSentencesAreNotConfusedForEachOther() {
        val declined = Connection.Failure.of(Connection.Failure.Said.declined("box.local"))
        val stopped = Connection.Failure.of(Connection.Failure.Said.stoppedWaiting("box.local"))
        assertNotEquals(declined, stopped)
        assertNotEquals(Connection.Failure.OTHER, declined)
        assertNotEquals(Connection.Failure.OTHER, stopped)
    }

    /**
     * And neither is recovered from the runner's name, which a person chooses.
     *
     * A runner labelled "Stopped waiting" is absurd and a runner reached at an
     * address containing "has not been trusted" is not something this app can
     * rule out — what matters is that the phrase the classifier looks for comes
     * from the sentence and not from the substitution in the middle of it.
     */
    @Test
    fun aRunnerNamedAfterAPhraseDoesNotChangeWhatTheSentenceMeans() {
        assertEquals(
            Connection.Failure.KEY_NOT_TRUSTED,
            Connection.Failure.of(Connection.Failure.Said.declined("Stopped waiting")),
        )
    }

    /**
     * A tunneled runner has no address at all, so its sentences are built out
     * of the label the granting device sent. `Runner.named` is what does that,
     * and a sentence about "" would name nothing.
     */
    @Test
    fun aTunneledRunnerIsNamedInBothSentences() {
        val tunneled = Runner(id = "r2", label = "Studio", reach = Reach.Tailcat("tok"), user = "me")
        assertTrue(Connection.Failure.Said.declined(tunneled.named).contains("Studio"))
        assertTrue(Connection.Failure.Said.stoppedWaiting(tunneled.named).contains("Studio"))
    }
}
