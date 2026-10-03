package com.farcooler.notify

/**
 * What [FarCoolerMessagingService.onMessageReceived] reads off a push: a task
 * notice, or an agent's or a decision's card. Pure, so the JVM tests can read
 * the relay's own FCM messages (`test/fixtures/contracts/push/fcm/`) with the
 * code the service runs (ov-121).
 *
 * [notificationTitle] and [notificationBody] are `RemoteMessage.notification`'s,
 * which a card Firebase draws carries and a card this app draws doesn't.
 */
sealed interface PushMessage {
    val title: String
    val body: String

    /** A task notice (ov-94), drawn by [TaskNotices.post]. */
    data class Task(val notice: TaskNotice, override val title: String, override val body: String) : PushMessage

    /**
     * An agent's card, or an old runner's decision. [terminal] is empty for a
     * decision; [task] and [runner] are set for a decision only.
     */
    data class Card(
        override val title: String,
        override val body: String,
        val terminal: String,
        val channel: String,
        val kind: String?,
        val task: String?,
        val runner: String?,
    ) : PushMessage

    companion object {
        /** `null` for a push with no title to show. */
        fun of(data: Map<String, String>, notificationTitle: String?, notificationBody: String?): PushMessage? {
            val title = data["title"] ?: notificationTitle ?: return null
            val body = data["body"] ?: notificationBody.orEmpty()
            TaskNotice.of(data)?.let { return Task(it, title, body) }
            // A decision names a task and no terminal: the tap opens its card.
            val kind = data[Notifier.PUSH_EXTRA_KIND]
            val task = data[Notifier.PUSH_EXTRA_TASK]?.takeIf { kind == Notifier.KIND_DECISION }
            return Card(
                title = title,
                body = body,
                terminal = data[Notifier.PUSH_EXTRA_TERMINAL].orEmpty(),
                // `blocked` is the state worth a high-importance channel;
                // anything else the daemon chose to send is news that can wait.
                // See the history at [NotificationCopy.channelForPush]: this
                // read was once `data["activity"]`, a key no producer sent.
                channel = NotificationCopy.channelForPush(data),
                kind = kind,
                task = task,
                runner = data[Notifier.PUSH_EXTRA_RUNNER]?.takeIf { task != null && it.isNotEmpty() },
            )
        }
    }
}
