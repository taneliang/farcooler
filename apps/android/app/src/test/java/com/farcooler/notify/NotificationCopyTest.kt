package com.farcooler.notify

import com.farcooler.model.Terminal
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Every sentence that can reach a lock screen, and who else writes it.
 *
 * Two halves, and the second is the one that matters. The first pins what this
 * app says about a pane — a dead turn does not say "finished", a finished one
 * quotes what the agent actually said, and no half-empty input produces a stray
 * separator. That much is table stakes.
 *
 * **What it does not prove is that this app and the runner say the same thing**,
 * and that is the failure that actually happens. One person gets one
 * notification about one pane, drawn either here from a fleet this phone polled
 * or by the daemon into a push when nobody was polling — and two casings of one
 * sentence is two notifications. So [theDaemonWritesTheSameSentences] reads
 * `crates/daemon/src/watch.rs` out of this checkout, finds `fn notification`,
 * and asserts every sentence this file produces appears there literally.
 * Rewording either end fails on the next `testInstrumentedUnitTest`, which is
 * the same shape `GeneratedFilesTest.theTwoListsAreTheSame` and `FleetDecodeTest`
 * use for the same class of drift.
 *
 * It fails loudly rather than skipping when the Rust cannot be found, for the
 * reason that file gives: a test that quietly passes when it cannot check
 * anything is the same as no test, and this is one repository — the Android
 * build already reads `../../scripts/version.sh` out of it.
 */
class NotificationCopyTest {

    private fun terminal(
        id: String = "t",
        title: String = "claude",
        activity: String? = null,
        turnFailed: Boolean? = null,
        blockedQuestion: String? = null,
        said: String? = null,
        feed: List<String>? = null,
    ) = Terminal(
        id = id,
        title = title,
        state = "running",
        activity = activity,
        turnFailed = turnFailed,
        blockedQuestion = blockedQuestion,
        said = said,
        feed = feed,
    )

    // ---- what it says ----

    /**
     * The bug this phase exists for, at the surface it costs most.
     *
     * `activity` says only that the turn ended; the daemon reads the agent's own
     * log and says which in `turnFailed`. A card claiming an agent finished when
     * its turn came back an error is the lie the row's green checkmark used to
     * tell — and this one arrives on a lock screen, where it decides whether
     * somebody gets up.
     */
    @Test
    fun `a turn that died never says finished`() {
        val dead = NotificationCopy.of(
            terminal(activity = "done", turnFailed = true, said = "I'll try that again."),
            workspace = "add-auth",
        )!!
        assertEquals("claude failed", dead.title)
        assertEquals("add-auth — ${NotificationCopy.DID_NOT_FINISH}", dead.body)
        assertFalse(dead.body.contains("I'll try that again."))
        // Still the quieter channel. A turn that failed has STOPPED, which is
        // news; it is not waiting on an answer, which is what the high-importance
        // channel is reserved for.
        assertEquals(Notifier.CHANNEL_DONE, dead.channel)
    }

    /** `turnFailed` is only ever a claim about a turn that ended. */
    @Test
    fun `a working agent carrying a stale failure flag is not announced as failed`() {
        assertNull(NotificationCopy.of(terminal(activity = "working", turnFailed = true), "w"))
        val blocked = NotificationCopy.of(
            terminal(activity = "blocked", turnFailed = true),
            workspace = "add-auth",
        )!!
        assertEquals("claude needs you", blocked.title)
    }

    @Test
    fun `a finished agent says what it finished`() {
        val done = NotificationCopy.of(
            terminal(activity = "done", said = "Both tests pass."),
            workspace = "add-auth",
        )!!
        assertEquals("claude finished", done.title)
        assertEquals("add-auth — Both tests pass.", done.body)
    }

    /**
     * The cut that matters. A feed entry is a wrapped ROW, so its last line is
     * the END of the window — which is how a turn that ended "More shit. An
     * industrial quantity of shit, shipped in carefully authorized batches to
     * avoid N+1 shits." reached a phone as "batches to avoid N+1 shits."
     */
    @Test
    fun `the opening of the message is quoted, not the end of the window`() {
        val done = NotificationCopy.of(
            terminal(
                activity = "done",
                said = "More shit, shipped in carefully authorized batches.",
                feed = listOf("More shit, shipped in carefully", "authorized batches."),
            ),
            workspace = "add-auth",
        )!!
        assertEquals("add-auth — More shit, shipped in carefully authorized batches.", done.body)
    }

    /**
     * A runner too old to send `said` still gets a body. The tail of the window
     * is a worse sentence than the head and a much better one than nothing.
     */
    @Test
    fun `an older runner falls back to the feed`() {
        val done = NotificationCopy.of(
            terminal(activity = "done", feed = listOf("Reading watch.rs.", "Ran the suite.")),
            workspace = "add-auth",
        )!!
        assertEquals("add-auth — Ran the suite.", done.body)
    }

    /** A turn can be all tool calls and say nothing. That is a real case. */
    @Test
    fun `a turn that said nothing is the workspace alone`() {
        assertEquals("add-auth", NotificationCopy.of(terminal(activity = "done"), "add-auth")!!.body)
        assertEquals(
            "add-auth",
            NotificationCopy.of(terminal(activity = "done", said = "   "), "add-auth")!!.body,
        )
    }

    @Test
    fun `a blocked agent asks its own question`() {
        val blocked = NotificationCopy.of(
            terminal(activity = "blocked", blockedQuestion = "Run `rm -rf build`?"),
            workspace = "add-auth",
        )!!
        assertEquals("claude needs you", blocked.title)
        assertEquals("add-auth — Run `rm -rf build`?", blocked.body)
        assertEquals(Notifier.CHANNEL_BLOCKED, blocked.channel)
    }

    /**
     * `blocked` is derived from a tmux SCREEN, so there is a scraped line some of
     * the time and nothing the rest of it.
     */
    @Test
    fun `a blocked agent with nothing scraped still says what to do`() {
        assertEquals(
            "add-auth — ${NotificationCopy.WAITING}",
            NotificationCopy.of(terminal(activity = "blocked"), "add-auth")!!.body,
        )
        assertEquals(
            "add-auth — ${NotificationCopy.WAITING}",
            NotificationCopy.of(terminal(activity = "blocked", blockedQuestion = " "), "add-auth")!!
                .body,
        )
    }

    /**
     * Only the two states that interrupt somebody. A working agent is the normal
     * case, and a product that buzzes for the normal case is one people turn off.
     */
    @Test
    fun `nothing else is worth a banner`() {
        for (state in listOf(null, "none", "idle", "working", "unknown", "nonsense")) {
            assertNull(state, NotificationCopy.of(terminal(activity = state), "add-auth"))
        }
    }

    // ---- the runner, and the joins ----

    /**
     * The fleet is several runners at once on this platform — see the
     * do-not-delete list — so a notification has to be able to say which. With
     * one connected its name is on every notification and says nothing.
     */
    @Test
    fun `the runner is named only when it adds something`() {
        val on = { runner: String ->
            NotificationCopy.of(terminal(activity = "done", said = "Both tests pass."), "add-auth", runner)!!.body
        }
        assertEquals("add-auth — Both tests pass.", on(""))
        assertEquals("add-auth — Both tests pass.", on("   "))
        assertEquals("add-auth — Both tests pass. · studio", on("studio"))
    }

    /**
     * Every half of the body can be missing, and none of them may leave a
     * separator behind. `— Both tests pass.` looks like a rendering bug, and a
     * lock screen is not where to debug one.
     */
    @Test
    fun `a missing half never leaves a stray separator`() {
        val nameless = terminal(activity = "done", said = "Both tests pass.")
        assertEquals("Both tests pass.", NotificationCopy.of(nameless, "")!!.body)
        assertEquals("Both tests pass. · studio", NotificationCopy.of(nameless, " ", "studio")!!.body)
        assertEquals("studio", NotificationCopy.of(terminal(activity = "done"), "", "studio")!!.body)
        assertEquals("", NotificationCopy.of(terminal(activity = "done"), "")!!.body)
    }

    // ---- the push ----

    /**
     * `status` is the daemon's own word for what it is announcing. Anything else
     * — including the nothing that arrives today — is news that can wait.
     */
    @Test
    fun `only a blocked push earns the high-importance channel`() {
        assertEquals(Notifier.CHANNEL_BLOCKED, NotificationCopy.channelFor("blocked"))
        assertEquals(Notifier.CHANNEL_DONE, NotificationCopy.channelFor("done"))
        assertEquals(Notifier.CHANNEL_DONE, NotificationCopy.channelFor("working"))
        assertEquals(Notifier.CHANNEL_DONE, NotificationCopy.channelFor(null))
    }

    /**
     * The name is read by, spelled the producer's way.
     *
     * This read was `data["activity"]` — a key no producer has ever sent under
     * any name, so the high-importance channel was unreachable from the push
     * path. Pinned against the relay's own type so that a rename there fails
     * here rather than going quiet on a phone.
     *
     * It is NOT on the FCM message today: `sendFcm` builds `data: { terminal }`
     * and nothing else. That is stated in `NotificationCopy.channelFor` rather
     * than pinned by a test, because a test asserting the absence of a key is a
     * test that fails on the day somebody fixes it.
     */
    @Test
    fun `the push keys are the relay's own`() {
        val relay = repositoryFile("services/relay/src/push.ts")
        assertTrue(
            "Payload.status is gone from the relay; FarCoolerMessagingService reads a key by " +
                "that name to choose a notification channel.",
            relay.contains("status?: string"),
        )
        assertTrue(
            "Payload.terminal is gone from the relay; it is the key a tapped push arrives " +
                "under — see Notifier.PUSH_EXTRA_TERMINAL.",
            relay.contains("terminal: string"),
        )
    }

    /**
     * Firebase draws the tray notification itself whenever this app is not in
     * the foreground, and reads the channel to draw it on out of the manifest by
     * name. A channel id nothing created is one Android silently substitutes for
     * — which would put the notification the whole push path exists for into a
     * channel called "Miscellaneous".
     */
    @Test
    fun `the manifest's default channel is one this app creates`() {
        val strings = repositoryFile("apps/android/app/src/main/res/values/strings.xml")
        val declared = Regex(
            "<string name=\"default_notification_channel_id\"[^>]*>([^<]*)</string>"
        ).find(strings)
        assertTrue("default_notification_channel_id is not declared any more", declared != null)
        assertEquals(Notifier.CHANNEL_DONE, declared!!.groupValues[1])

        val manifest = repositoryFile("apps/android/app/src/main/AndroidManifest.xml")
        assertTrue(
            "The manifest no longer names Firebase's default channel, so a push drawn by the " +
                "system lands wherever the SDK decides.",
            manifest.contains("com.google.firebase.messaging.default_notification_channel_id"),
        )
    }

    // ---- the drift ----

    /**
     * Every sentence, checked against the runner that writes the other copy.
     *
     * `format!("{label} needs you")` in the Rust is matched by building the
     * announcement with the literal label `{label}`, so the assertion is against
     * the whole rendered title rather than a fragment of it — a title that grew a
     * word on one side and not the other fails here.
     */
    @Test
    fun theDaemonWritesTheSameSentences() {
        val notification = daemonNotificationFunction()

        for (state in listOf("blocked", "done")) {
            for (failed in listOf(false, true)) {
                val title = NotificationCopy.of(
                    terminal(title = "{label}", activity = state, turnFailed = failed),
                    workspace = "",
                )!!.title
                assertTrue(
                    "The daemon no longer writes the title \"$title\". One person reads either " +
                        "this app's banner or the daemon's push about one pane; two wordings is " +
                        "two notifications. See crates/daemon/src/watch.rs, fn notification.",
                    notification.contains("\"$title\""),
                )
            }
        }

        for (sentence in listOf(NotificationCopy.WAITING, NotificationCopy.DID_NOT_FINISH)) {
            assertTrue(
                "The daemon no longer writes \"$sentence\". If it was reworded, reword it here " +
                    "too rather than deleting this check.",
                notification.contains("\"$sentence\""),
            )
        }
    }

    /**
     * The join, from the same source. `Quoted::body` is what puts the workspace
     * in front of whatever there is to say, and this file's `body` is a port of
     * it — including which separator, which is the visible half.
     */
    @Test
    fun `the workspace is joined the same way on both sides`() {
        val watch = repositoryFile("crates/daemon/src/watch.rs")
        assertTrue(
            "Quoted::body no longer joins with an em dash. NotificationCopy.body is a port of " +
                "it and the two land on one lock screen.",
            watch.contains("format!(\"{workspace} — {text}\")"),
        )
    }

    /**
     * `fn notification` in `crates/daemon/src/watch.rs`, up to the closing brace
     * at its own indentation.
     *
     * Bounded rather than searched whole, so a sentence that moved OUT of the
     * notification builder and survives elsewhere in a 5,000-line file does not
     * keep this passing.
     */
    private fun daemonNotificationFunction(): String {
        val watch = repositoryFile("crates/daemon/src/watch.rs")
        val start = watch.indexOf("\nfn notification(")
        assertTrue(
            "fn notification is not declared in crates/daemon/src/watch.rs any more. If the " +
                "push copy moved, point this at where it went rather than deleting the check.",
            start >= 0,
        )
        val end = watch.indexOf("\n}\n", start)
        assertTrue("fn notification is unterminated", end > start)
        return watch.substring(start, end)
    }

    /** A file in this checkout, found by walking up from wherever Gradle runs. */
    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError(
            "Could not find $relative above ${System.getProperty("user.dir")}. This test is the " +
                "only thing holding this app's notification copy to the runner's; if the file " +
                "has moved, point this at where it went rather than deleting the check."
        )
    }
}
