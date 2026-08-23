package com.farcooler.notify

import android.app.PendingIntent
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import com.farcooler.R
import com.farcooler.ui.MainActivity
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

/**
 * A push from the relay, about an agent on a runner this phone is not watching.
 *
 * The one case the product exists for: the app is closed, the phone is asleep,
 * and an agent three time zones away has stopped and is waiting for an answer.
 * Nothing local can know that — the daemon tells the relay, and the relay tells
 * this.
 *
 * Rendered here rather than handed to [Notifier], deliberately: [Notifier]
 * decides what is worth announcing from a fleet it just polled, and there is no
 * fleet here — the app is not running. The daemon already made that judgement
 * before it sent anything, so this draws what it was told and adds no opinion
 * of its own.
 *
 * ## When this runs, which is not when you would think
 *
 * The relay sends a message with a `notification` block — `sendFcm` in
 * `services/relay/src/push.ts` — and Firebase's rule for those is that the
 * SYSTEM draws the tray notification whenever the app is backgrounded or dead,
 * and calls [onMessageReceived] only while it is in the foreground. So every
 * careful thing below happens in the one case the local [Notifier] already
 * covers, and the case this whole path exists for — a phone asleep in a pocket —
 * is drawn by Firebase out of the manifest's defaults.
 *
 * That is why the manifest names a default channel and a default icon, and why
 * [Notifier.PUSH_EXTRA_TERMINAL] exists: those three are the only say this app
 * gets over the notification it most wants to get right. Making this method the
 * one that draws it needs the relay to send a data-only message, which is a
 * change with its own costs — Firebase does not wake a force-stopped app for
 * one — and is not this app's to make.
 */
class FarCoolerMessagingService : FirebaseMessagingService() {

    /**
     * A new address for this device.
     *
     * FCM rotates tokens, and a stale one is a push that reaches nobody. The
     * app cannot file it from here — there may be no signed-in account in this
     * process — so it is recorded and sent on next launch, which is the same
     * hold-until-signed-in shape `PushRegistration` already has.
     */
    override fun onNewToken(token: String) {
        getSharedPreferences(PREFS, MODE_PRIVATE).edit().putString(KEY_PENDING_TOKEN, token).apply()
    }

    override fun onMessageReceived(message: RemoteMessage) {
        val data = message.data
        val title = data["title"] ?: message.notification?.title ?: return
        val body = data["body"] ?: message.notification?.body.orEmpty()
        val terminal = data[Notifier.PUSH_EXTRA_TERMINAL].orEmpty()
        // `blocked` is the state worth a high-importance channel; anything else
        // the daemon chose to send is news that can wait.
        //
        // This read used to be `data["activity"]`, a key no producer has ever
        // sent under any name — so the high-importance channel that exists for
        // "an agent has stopped and is waiting for you" was unreachable from
        // the push path, and had been since it was written. It is spelled the
        // producer's way now; see [NotificationCopy.channelFor] for what still
        // has to change on the relay before it carries anything.
        val channel = NotificationCopy.channelFor(data[Notifier.PUSH_EXTRA_STATUS])

        val intent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            if (terminal.isNotEmpty()) putExtra(Notifier.EXTRA_TERMINAL, terminal)
        }
        val pending = PendingIntent.getActivity(
            this,
            terminal.hashCode(),
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

        val notification = NotificationCompat.Builder(this, channel)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(body)
            // The same expansion the local banner gets, for the same reason:
            // the daemon's subtitle carries the agent's own last words, and one
            // lock-screen line is not enough of them to decide anything by.
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setAutoCancel(true)
            // Keyed by terminal, so the app's own banner about the same pane
            // replaces this one rather than sitting beside it when the phone is
            // picked up and the fleet is polled again.
            .setGroup(terminal.ifEmpty { "farcooler" })
            .setContentIntent(pending)
            .build()

        runCatching {
            NotificationManagerCompat.from(this)
                .notify(terminal.ifEmpty { title }.hashCode(), notification)
        }
    }

    companion object {
        const val PREFS = "farcooler.push"
        const val KEY_PENDING_TOKEN = "pendingToken"
    }
}
