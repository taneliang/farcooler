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
 * A push from the relay, about an agent on a machine this phone is not watching.
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
        val terminal = data["terminal"].orEmpty()
        // `blocked` is the state worth a high-importance channel; anything else
        // the daemon chose to send is news that can wait.
        val blocked = data["activity"] == "blocked"

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

        val notification = NotificationCompat.Builder(
            this,
            if (blocked) Notifier.CHANNEL_BLOCKED else Notifier.CHANNEL_DONE,
        )
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(body)
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
