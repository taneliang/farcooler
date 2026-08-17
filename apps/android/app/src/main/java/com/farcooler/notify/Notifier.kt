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
     * What was last announced per terminal, so a state that persists is
     * announced once. The fleet is polled, so the same `done` arrives over and
     * over; being told twice that the same agent finished is how people learn
     * to ignore notifications.
     *
     * Concurrent because every runner polls on its own coroutine and they all
     * report here — with three runners connected this is three writers, and a
     * plain map would eventually corrupt rather than merely race.
     */
    private val announced = java.util.concurrent.ConcurrentHashMap<String, AgentActivity>()

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
                "Agents that finished",
                NotificationManager.IMPORTANCE_DEFAULT,
            ).apply {
                description = "An agent finished the task you gave it."
            }
        )
    }

    /** Announce a change, if it is worth announcing. */
    fun report(terminal: Terminal, workspace: String, runner: String) {
        val activity = terminal.agent
        val previous = announced[terminal.id]
        announced[terminal.id] = activity

        if (!settings.notifyOnAttention.value) return
        if (!activity.wantsAttention) return
        if (activity == previous) return
        if (activity == AgentActivity.DONE && !settings.notifyOnDone.value) return
        // The pane you are already reading is the one case a banner is noise.
        if (isForeground && terminal.id == visibleTerminal) return
        if (!canPost()) return

        val title: String
        val body: String
        val channel: String
        when (activity) {
            AgentActivity.BLOCKED -> {
                title = "${terminal.label} needs you"
                body = "$workspace — waiting for your answer"
                channel = CHANNEL_BLOCKED
            }

            AgentActivity.DONE -> {
                title = "${terminal.label} finished"
                body = workspace
                channel = CHANNEL_DONE
            }

            else -> return
        }

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
            // The runner, only when it adds something. With one runner
            // connected its name is on every notification and says nothing;
            // with three it is the first thing you need.
            .setContentText(if (runner.isBlank()) body else "$body · $runner")
            .setAutoCancel(true)
            // Grouped by terminal so a later state replaces the earlier
            // notification for the same one rather than stacking up.
            .setGroup(terminal.id)
            .setContentIntent(pending)
            .setCategory(
                if (activity == AgentActivity.BLOCKED) NotificationCompat.CATEGORY_CALL
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
        const val CHANNEL_DONE = "agents.done"
        const val EXTRA_TERMINAL = "com.farcooler.terminal"
    }
}
