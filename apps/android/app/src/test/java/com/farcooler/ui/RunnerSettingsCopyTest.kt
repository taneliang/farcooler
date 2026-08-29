package com.farcooler.ui

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * What the settings screen says about the calls it cannot make.
 *
 * Two of its reads — `adapter.list` and `repository_root.list` — are
 * `Scope::HostAdmin` while this app enrolls at `control`, as are every write
 * under Branches, Themes and Agents. A key added to `authorized_keys` by hand
 * carries no forced command, so `Session::granted` in
 * `crates/daemon/src/main.rs` reads it as host_admin; a key Far Cooler enrolled
 * carries `--scope control`. Two phones with identical app settings, two
 * different answers.
 *
 * **The app can now tell those two phones apart, and only in advance.** The
 * handshake carries `ServerHello.granted_scope`, computed by the daemon from
 * the session's real grant, and `DaemonBuild.mayAdministerRunner` reads it — so
 * a control this connection may not use is dimmed before it is pressed, with
 * [restrictedSentence] saying why and what to do. That is what the screen knows
 * ahead of time.
 *
 * **What a failure MEANS is still unknowable, and that half has not moved.** A
 * runner asleep, a daemon too old for the method, a socket that went away and a
 * refusal all arrive at [deniedSentence] looking identical, so it names what is
 * missing and blames nothing — exactly as before. The two sentences are about
 * two different moments and neither may be rewritten into the other.
 *
 * It is also why `adapters()` no longer swallows its failure. Every runner has
 * adapters, so the empty list it used to answer with was a false statement made
 * in the runner's name.
 *
 * Both sentences are held to the same banned words. Two of those are the wire's
 * own vocabulary, which means nothing to the person reading the screen; the
 * rest are claims about the runner this app has no business making even now
 * that it can predict the outcome.
 */
class RunnerSettingsCopyTest {

    /**
     * The words no sentence on this screen may use, whatever it is about.
     *
     * `scope` and `host_admin` are the wire's, and spelling them here would ask
     * somebody to learn `authorized_keys` to read a phone screen. `denied`,
     * `permission` and `control` are claims about the runner: the first two
     * name a refusal this app is not the one making, and the third is the word
     * for one rung of a ladder nothing else in this UI mentions.
     *
     * One list rather than one per test, so that a sentence added later is
     * held to the same bar by construction instead of by whoever remembers.
     */
    private val banned = listOf("scope", "denied", "permission", "host_admin", "control")

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
        for (word in banned) {
            assertFalse(word, folders.lowercase().contains(word))
        }
    }

    /**
     * The dimmed reason says what to do, in words nobody has to look up.
     *
     * This one CAN name a cause — the grant arrives in the handshake, so the
     * screen knows before anything is pressed — and it still may not use the
     * vocabulary that cause is written in. `authorized_keys` is not a thing a
     * phone screen may expect anybody to have read.
     *
     * The way forward has to be a real one, which is why it names the runner's
     * own command line and not the Mac app: `AddDeviceView` adds a phone at
     * exactly the access it already has, so a sentence pointing there would
     * send somebody around a circle and back to the same dimmed buttons.
     */
    @Test
    fun `the dimmed reason offers a way forward and no wire words`() {
        val reason = restrictedSentence().lowercase()
        for (word in banned) {
            assertFalse(word, reason.contains(word))
        }
        // Something to do, and somewhere that actually does it.
        assertTrue(reason.contains("add it again"))
        assertTrue(reason.contains("command line"))
        // Not the Mac app. It adds a phone at the same access, so naming it
        // here would be a loop with a dead end at the end of it.
        assertFalse(reason.contains("mac app"))
    }

    /**
     * The two sentences stay two sentences.
     *
     * They are about different moments — one before a call, one after — and the
     * standing temptation is to collapse them once the app can predict the
     * common case. It must not: a call can still fail for reasons that have
     * nothing to do with what this device was granted, and [deniedSentence]
     * covering that is the whole reason it claims nothing.
     */
    @Test
    fun `the dimmed reason is not the failure sentence`() {
        assertFalse(restrictedSentence() == deniedSentence("agents"))
        // Untouched by this: it still says what is missing and never why.
        assertTrue(deniedSentence("agents") == "Couldn’t read this runner’s agents.")
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
