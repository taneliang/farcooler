package com.farcooler.ui

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What the rendezvous section is allowed to say.
 *
 * The field is a recovery valve for one day — the day the rendezvous the app
 * ships with stops answering — and its copy has two jobs that pull against each
 * other. It has to explain the mechanism well enough that the effect of typing
 * something is predictable, and it must never read as an invitation. **Nothing
 * in this product says anyone should run their own rendezvous, and this is the
 * screen most likely to become the place that implies it.**
 *
 * The third job is the phishing one. A setting that moves where a phone looks
 * for its runners is exactly the setting a caller claiming to be support would
 * talk somebody through, so the sentence saying that will never happen is not
 * decoration and is not optional.
 *
 * Held here rather than by reading the screen, because these are constants the
 * composable renders and this app has no Compose test harness. What that leaves
 * unproven is placement, which is stated in `RendezvousSection`'s own comment
 * and cannot be asserted from a JVM test.
 */
class RendezvousCopyTest {
    private val prose = listOf(
        RENDEZVOUS_EXPLANATION,
        RENDEZVOUS_BOTH_ENDS,
        RENDEZVOUS_RECONNECT,
        RENDEZVOUS_FOOTER,
    )

    /**
     * The words that would turn a recovery valve into a feature.
     *
     * "Your own" and "self-host" invite; "derp", "derpmap" and "tailcat" are the
     * implementation's vocabulary and mean nothing to somebody reading a phone
     * screen — spelling them would ask a reader to go and learn a protocol to
     * understand a settings row. The placeholder inside the text field is a URL
     * rather than prose and is deliberately not held to this.
     */
    private val banned = listOf("your own", "self-host", "self host", "derp", "tailcat", "tailscale")

    @Test
    fun `no sentence invites anybody to run a rendezvous or names the protocol`() {
        for (sentence in prose) {
            for (word in banned) {
                assertFalse("$word in: $sentence", sentence.lowercase().contains(word))
            }
        }
    }

    /**
     * The footer says what to do and that support will never ask.
     *
     * Both halves. "Leave this empty" alone is advice somebody talks you out of;
     * naming the call that would do the talking is what makes it hard to.
     */
    @Test
    fun `the footer says to leave it alone and that support will never ask`() {
        val footer = RENDEZVOUS_FOOTER.lowercase()
        assertTrue(footer, footer.contains("leave this empty"))
        assertTrue(footer, footer.contains("support will never ask"))
    }

    /**
     * The explanation says what a rendezvous does and why the field exists.
     *
     * Without the first half, the effect of typing something is unpredictable.
     * Without the second, the field looks like a preference rather than a way
     * out of a revocation.
     */
    @Test
    fun `the explanation gives the mechanism and the reason`() {
        val text = RENDEZVOUS_EXPLANATION.lowercase()
        assertTrue(text, text.contains("no address"))
        assertTrue(text, text.contains("meet"))
        assertTrue(text, text.contains("without an app update"))
    }

    /**
     * The custom-rendezvous line says both ends have to agree.
     *
     * A runner's rendezvous is an environment variable its installer sets,
     * `FARCOOLER_DERP_MAP`, and a phone cannot change it — deliberately, so a
     * runner cannot be moved onto a rendezvous by whoever is dialing it. A phone
     * pointed somewhere its runners are not simply never meets them, and the
     * symptom is a connection that times out saying nothing. This sentence is
     * the only place in the app that warns about it.
     */
    @Test
    fun `the custom rendezvous warning says runners have to agree`() {
        val text = RENDEZVOUS_BOTH_ENDS.lowercase()
        assertTrue(text, text.contains("runners have to be set to the same one"))
        assertTrue(text, text.contains("won’t be reachable"))
    }

    /**
     * Changing it tells you to reconnect.
     *
     * The setting is read when a connection is opened, so a session already
     * running is still meeting at the old place — see `Connection.rendezvous`.
     * Without this line the change would look like it did nothing.
     */
    @Test
    fun `changing the rendezvous asks for a reconnect`() {
        assertTrue(RENDEZVOUS_RECONNECT.lowercase().contains("reconnect"))
    }
}
