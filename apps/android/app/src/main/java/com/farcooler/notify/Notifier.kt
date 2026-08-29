package com.farcooler.notify

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.farcooler.R
import com.farcooler.data.Settings
import com.farcooler.model.AgentActivity
import com.farcooler.model.Terminal
import com.farcooler.ui.MainActivity

/**
 * Telling you when an agent needs you.
 *
 * The Mac's and iOS's `Notifier`, ported — same two states, same rule about
 * which are worth announcing. Only a BLOCKED agent waiting on you and a DONE
 * one are ever announced: working agents are the normal case, and a product
 * that buzzes for the normal case is one people turn off, after which it cannot
 * tell them the thing that mattered.
 *
 * The daemon decides those states, so this works identically for an agent on a
 * runner in the next room and one across the world. That is also what makes
 * the push path a delivery change rather than a rethink.
 *
 * ## Two channels, not one
 *
 * Android lets a person silence one kind of notification without silencing the
 * app, and these two kinds are genuinely different: an agent that is BLOCKED
 * has stopped and stays stopped until answered, which is worth a sound and a
 * place above the fold; an agent that FINISHED is news that can wait for the
 * next time the phone is picked up. One channel would force the same answer for
 * both, and the answer people give to "too noisy" is to turn everything off.
 */
class Notifier(private val context: Context, private val settings: Settings) {

    /**
     * The terminal on screen.
     *
     * A banner about this pane is suppressed — notifications DO show while the
     * app is open, because you are usually looking at one agent while another
     * is the one that got stuck, and swallowing every foreground banner would
     * hide exactly that. The one case it is noise is being told about the pane
     * you are already reading.
     */
    var visibleTerminal: String? = null

    var isForeground: Boolean = true

    /**
     * What was last announced about one terminal.
     *
     * The activity and whether the turn died, not the activity alone, because
     * the two are one piece of news: a turn that failed and one that succeeded
     * are both `done`, and a record that kept only `done` could not tell a
     * second announcement from a repeat of the first. The daemon fills
     * `turn_failed` and moves the activity under one lock in one pass — see
     * `crates/daemon/src/watch.rs` — so today they always arrive together and
     * this costs nothing; the day they do not, a failure is announced instead
     * of swallowed.
     *
     * Deliberately NOT the [Announcement] itself. A blocked agent's body
     * carries a question scraped off a tmux screen, and a line that reflows
     * would then read as new news about the same stopped agent.
     */
    private data class Announced(val activity: AgentActivity, val failed: Boolean)

    /**
     * What was last announced per terminal, so a state that persists is
     * announced once. The fleet is polled, so the same `done` arrives over and
     * over; being told twice that the same agent finished is how people learn
     * to ignore notifications.
     *
     * Concurrent because every runner polls on its own coroutine and they all
     * report here — with three runners connected this is three writers, and a
     * plain map would eventually corrupt rather than merely race.
     */
    private val announced = java.util.concurrent.ConcurrentHashMap<String, Announced>()

    private val manager = NotificationManagerCompat.from(context)

    fun createChannels() {
        val system = context.getSystemService(NotificationManager::class.java) ?: return
        system.createNotificationChannel(
            NotificationChannel(
                CHANNEL_BLOCKED,
                "Agents that need you",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "An agent has stopped and is waiting for your answer."
            }
        )
        system.createNotificationChannel(
            NotificationChannel(
                CHANNEL_DONE,
                // "or failed", because this channel has always carried both: a
                // turn that failed is reported as `done` with failure beside
                // it, so muting this is muting the pair, and the name has to
                // say so before somebody mutes it expecting otherwise.
                //
                // The ID does not move. `createNotificationChannel` on an
                // existing id updates the name and description in place, while
                // a NEW id resets everyone's mute back to default — silently
                // un-silencing people who had turned this off.
                "Agents that finished or failed",
                NotificationManager.IMPORTANCE_DEFAULT,
            ).apply {
                description =
                    "An agent finished the task you gave it, or its last turn didn’t finish."
            }
        )
    }

    /** Announce a change, if it is worth announcing. */
    fun report(terminal: Terminal, workspace: String, runner: String) {
        val activity = terminal.agent
        val news = Announced(activity, terminal.turnDidFail)
        val previous = announced.put(terminal.id, news)

        if (!settings.notifyOnAttention.value) return
        if (!activity.wantsAttention) return
        if (news == previous) return
        if (activity == AgentActivity.DONE && !settings.notifyOnDone.value) return
        // The pane you are already reading is the one case a banner is noise.
        //
        // The local half of the judgement. Its other half is a claim made to
        // the RUNNER — `Connection.reportWatching` — because the push that
        // arrives when the phone is asleep is not drawn here and this register
        // is not something the daemon can read.
        if (isForeground && terminal.id == visibleTerminal) return
        if (!canPost()) return

        // Every sentence, in one place that needs no `Context` and is held to
        // the daemon's own wording by a test. See [NotificationCopy].
        val (title, body, channel) = NotificationCopy.of(terminal, workspace, runner) ?: return

        val intent = Intent(context, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            putExtra(EXTRA_TERMINAL, terminal.id)
        }
        val pending = PendingIntent.getActivity(
            context,
            terminal.id.hashCode(),
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

        val notification = NotificationCompat.Builder(context, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(body)
            // A lock screen gives a body one line. The agent's own last words
            // are cut to about 120 characters by the daemon — see
            // `farcooler_core::feed::SAID_WIDTH` — which is more than one line
            // and exactly the part worth expanding to read.
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setAutoCancel(true)
            // Grouped by terminal so a later state replaces the earlier
            // notification for the same one rather than stacking up.
            .setGroup(terminal.id)
            .setContentIntent(pending)
            .setCategory(
                if (channel == CHANNEL_BLOCKED) NotificationCompat.CATEGORY_CALL
                else NotificationCompat.CATEGORY_STATUS
            )
            .build()

        runCatching { manager.notify(terminal.id.hashCode(), notification) }
    }

    /**
     * Forget a terminal that no longer exists, so a reused id cannot inherit
     * the announcement history of the terminal it replaced.
     */
    fun forget(terminalId: String) {
        announced.remove(terminalId)
        manager.cancel(terminalId.hashCode())
    }

    private fun canPost(): Boolean =
        ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED

    companion object {
        const val CHANNEL_BLOCKED = "agents.blocked"

        /**
         * Also named in `AndroidManifest.xml` as Firebase's default channel,
         * and the two must agree — see the meta-data there for why a push this
         * process never sees still has to land in a channel somebody can name.
         */
        const val CHANNEL_DONE = "agents.done"

        const val EXTRA_TERMINAL = "com.farcooler.terminal"

        /**
         * What a push calls the same thing.
         *
         * Firebase draws the tray notification itself when this app is not in
         * the foreground — which is the case the push exists for — and puts the
         * message's `data` keys into the launch intent VERBATIM. So a tapped
         * push arrives under the relay's spelling, not this app's, and
         * `MainActivity` has to try both or the deep link is silently dropped
         * for exactly the notification that mattered most. `"terminal"` is
         * `Payload.terminal` in `services/relay/src/push.ts`.
         */
        const val PUSH_EXTRA_TERMINAL = "terminal"

        /**
         * What the daemon calls the thing it is announcing: `"blocked"`,
         * `"done"`, `"working"`. `Notice.status` in
         * `crates/daemon/src/watch.rs`, `Payload.status` in the relay.
         */
        const val PUSH_EXTRA_STATUS = "status"
    }
}
