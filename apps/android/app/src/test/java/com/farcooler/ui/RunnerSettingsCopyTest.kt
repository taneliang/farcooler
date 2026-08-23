package com.farcooler.ui

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What the settings screen says when a runner would not answer a section.
 *
 * Two of its reads — `adapter.list` and `repository_root.list` — are
 * `Scope::HostAdmin` while this app enrolls at `control`, and **the app cannot
 * tell in advance which answer it will get.** A key added to `authorized_keys`
 * by hand carries no forced command, so `Session::granted` in
 * `crates/daemon/src/main.rs` reads it as host_admin; a key Far Cooler enrolled
 * carries `--scope control`. Two phones with identical settings, two different
 * answers, and nothing on this side to distinguish them — which is why the
 * sentence below claims neither.
 *
 * It is also why `adapters()` no longer swallows its failure. Every runner has
 * adapters, so the empty list it used to answer with was a false statement made
 * in the runner's name.
 */
class RunnerSettingsCopyTest {

    /**
     * The sentence says what is missing and never why.
     *
     * A runner asleep, a daemon too old for the method, a socket that went away
     * and a scope denial all arrive at this line looking identical. The runner's
     * own words go in the box underneath — the [com.farcooler.model.Trouble]
     * split — so nothing is lost by this half not guessing.
     */
    @Test
    fun `a section that would not answer names itself and blames nothing`() {
        val folders = deniedSentence("watched folders")
        assertTrue(folders.contains("watched folders"))
        // No diagnosis. "Denied", "permission" and "scope" would each be a claim
        // this side cannot support, and "scope" is a wire word besides.
        for (word in listOf("scope", "denied", "permission", "host_admin", "control")) {
            assertFalse(word, folders.lowercase().contains(word))
        }
    }

    /**
     * The note under Watched folders blames the client, not the runner.
     *
     * The absence of a Remove button on that section is the only thing on this
     * screen that is NOT a scope question, and reading it as one would send
     * somebody to widen a scope that changes nothing. It used to be that no app
     * could make the call at all — `Session::remove_repository_root` sent no
     * `TypedConfirmation` and `crates/daemon/src/rpc.rs` refuses without one,
     * before scope is looked at. That is fixed, and the limit is now this
     * screen's own: the runner wants the folder's name typed back and there is
     * nowhere here to type it. Either way the sentence has to say "this app" and
     * has to name somewhere that works.
     */
    @Test
    fun `the watched-folders note names the client and an alternative`() {
        val note = rootRemovalNote()
        assertTrue(note.contains("this app"))
        assertTrue(note.contains("Mac app"))
        // iOS collects the name in a sheet now, so it belongs in the list of
        // places this can be done. Leaving it out would send somebody to a Mac
        // they may not have.
        assertTrue(note.contains("iPhone app"))
        // Not a refusal by the runner. Every other missing control on this
        // screen is the runner's answer and this one is not, which is the only
        // thing this sentence is for.
        assertFalse(note.lowercase().contains("runner won"))
        assertFalse(note.lowercase().contains("not allowed"))
    }

    /**
     * One sentence, parameterized — not one per section.
     *
     * The app knows exactly the same amount about each of these, and two
     * hand-written sentences would eventually claim it knew more about one.
     */
    @Test
    fun `both host-admin sections get the same sentence with their own name in it`() {
        val agents = deniedSentence("agents")
        val folders = deniedSentence("watched folders")
        assertTrue(agents.contains("agents"))
        assertFalse(agents == folders)
        assertTrue(agents.replace("agents", "X") == folders.replace("watched folders", "X"))
        // The same words the base picker uses for the same shape of failure —
        // "Couldn’t read this project’s branches." Two sentences for one kind of
        // silence is how one screen comes to sound like two.
        assertTrue(agents.startsWith("Couldn’t read"))
    }
}
