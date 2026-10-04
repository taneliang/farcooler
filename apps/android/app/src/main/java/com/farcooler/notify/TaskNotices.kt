package com.farcooler.notify

import android.Manifest
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.RemoteInput
import androidx.core.content.ContextCompat
import com.farcooler.R
import com.farcooler.ui.MainActivity
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive
import java.util.concurrent.ConcurrentHashMap

/**
 * A task notice, as a push's `data` carries it (ov-94): `kind=task`, the
 * task's key, the runner it's on, its class, its id and a decision's options.
 *
 * The runner decided, worded and named it; this app draws what it was told.
 * `options` crosses as a JSON list in a string, because FCM data is strings.
 */
data class TaskNotice(
    val key: String,
    val runner: String?,
    val event: String?,
    val noticeId: String?,
    val options: List<String>,
) {
    companion object {
        /**
         * `null` unless [data] is a task notice naming a task. A legacy
         * decision carrying `event` is one too: the runner sends status
         * decisions that way for one stable release (ov-94).
         */
        fun of(data: Map<String, String>): TaskNotice? {
            val kind = data[Notifier.PUSH_EXTRA_KIND]
            val legacy = kind == Notifier.KIND_DECISION && data[TaskNotices.EXTRA_EVENT] == "decision"
            if (kind != TaskNotices.KIND_TASK && !legacy) return null
            val key = data[Notifier.PUSH_EXTRA_TASK]?.takeIf { it.isNotEmpty() } ?: return null
            return TaskNotice(
                key = key,
                runner = data[Notifier.PUSH_EXTRA_RUNNER]?.takeIf { it.isNotEmpty() },
                event = data[TaskNotices.EXTRA_EVENT]?.takeIf { it.isNotEmpty() },
                noticeId = data[TaskNotices.EXTRA_NOTICE_ID]?.takeIf { it.isNotEmpty() },
                options = optionsOf(data[TaskNotices.EXTRA_OPTIONS]),
            )
        }

        private fun optionsOf(raw: String?): List<String> {
            raw ?: return emptyList()
            val list = runCatching { Json.parseToJsonElement(raw) as? JsonArray }.getOrNull() ?: return emptyList()
            return list.mapNotNull { (it as? JsonPrimitive)?.takeIf { p -> p.isString }?.jsonPrimitive?.contentOrNull }
        }
    }
}

/**
 * The five task classes, their channels and switches, and a decision's card.
 *
 * Each class has its own channel, so the phone's own per-channel switch and the
 * app's agree; the relay names the same channel for a card Firebase draws
 * (`androidChannel` in `services/relay/src/push.ts`).
 */
object TaskNotices {
    /** In the order Settings lists them. `TaskNoticeEvent` on Apple. */
    val EVENTS = listOf("decision", "review", "blocked", "done", "new")

    const val KIND_TASK = "task"
    const val EXTRA_EVENT = "event"
    const val EXTRA_NOTICE_ID = "noticeId"
    const val EXTRA_OPTIONS = "options"

    /** The answer actions, and the extras [AnswerReceiver] reads. */
    const val ACTION_OPTION = "com.farcooler.notify.ANSWER_OPTION"
    const val ACTION_TEXT = "com.farcooler.notify.ANSWER_TEXT"
    const val EXTRA_INDEX = "com.farcooler.notify.index"
    const val EXTRA_KEY = "com.farcooler.notify.key"
    const val EXTRA_RUNNER = "com.farcooler.notify.runner"
    const val EXTRA_ID = "com.farcooler.notify.noticeId"
    const val EXTRA_OPTION_LIST = "com.farcooler.notify.options"
    const val REMOTE_INPUT = "com.farcooler.notify.answer"

    /** As the Needs You rows draw them. */
    const val OPTION_LIMIT = 3

    fun channelFor(event: String): String = "tasks.$event"

    /** The switch's name in Settings. */
    fun title(event: String): String = when (event) {
        "decision" -> "Needs a decision"
        "review" -> "Ready for review"
        "blocked" -> "Blocked"
        "done" -> "Done"
        "new" -> "New task"
        else -> event
    }

    /** Needs a Decision, Ready for Review and Blocked are on until turned off. */
    fun onByDefault(event: String): Boolean = event == "decision" || event == "review" || event == "blocked"

    /**
     * What registration sends the relay as `notifyEvents`: the classes [isOn]
     * says are on, or none with the master switch off.
     */
    fun events(master: Boolean, isOn: (String) -> Boolean): List<String> =
        if (master) EVENTS.filter(isOn) else emptyList()

    /**
     * The answer an action sends, or null for one that sends nothing: an
     * option this card doesn't have, or a typed answer that's only space.
     */
    fun answerOf(action: String?, index: Int, options: List<String>, typed: CharSequence?): String? = when (action) {
        ACTION_OPTION -> options.take(OPTION_LIMIT).getOrNull(index)
        ACTION_TEXT -> typed?.toString()?.trim()?.takeIf { it.isNotEmpty() }
        else -> null
    }

    /**
     * Draw a task notice this app was handed: a push in the foreground, or a
     * decision, which the relay sends as data so it can carry buttons.
     *
     * Posted under `(noticeId, 0)`, the pair Firebase uses for a card it drew
     * with that tag, so either replaces the other: one card per task.
     */
    fun post(context: Context, notice: TaskNotice, title: String, body: String) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) return
        val tag = notice.noticeId ?: "task:${notice.key}"
        val event = notice.event ?: "decision"
        val open = Intent(context, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            putExtra(Notifier.PUSH_EXTRA_KIND, KIND_TASK)
            putExtra(Notifier.PUSH_EXTRA_TASK, notice.key)
            notice.runner?.let { putExtra(Notifier.PUSH_EXTRA_RUNNER, it) }
        }
        val builder = NotificationCompat.Builder(context, channelFor(event))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText(body)
            .setStyle(NotificationCompat.BigTextStyle().bigText(body))
            .setAutoCancel(true)
            .setGroup(tag)
            .setContentIntent(
                PendingIntent.getActivity(
                    context, tag.hashCode(), open,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                )
            )
        if (event == "decision") {
            notice.options.take(OPTION_LIMIT).forEachIndexed { index, option ->
                builder.addAction(
                    answerAction(option, answerIntent(context, notice, tag, ACTION_OPTION, index, mutable = false), null)
                )
            }
            val input = RemoteInput.Builder(REMOTE_INPUT).setLabel("Your answer").build()
            builder.addAction(
                answerAction("Answer", answerIntent(context, notice, tag, ACTION_TEXT, -1, mutable = true), input)
            )
        }
        runCatching { NotificationManagerCompat.from(context).notify(tag, 0, builder.build()) }
    }

    /**
     * One answer button. Each asks for the device to be unlocked first: an
     * answer moves somebody's work, and anyone holding a locked phone
     * shouldn't be able to send one (as iOS's `.authenticationRequired`).
     */
    fun answerAction(title: String, intent: PendingIntent?, input: RemoteInput?): NotificationCompat.Action {
        val action = NotificationCompat.Action.Builder(0, title, intent)
            .setAuthenticationRequired(true)
            .setAllowGeneratedReplies(false)
        input?.let { action.addRemoteInput(it) }
        return action.build()
    }

    private fun answerIntent(
        context: Context, notice: TaskNotice, tag: String, action: String, index: Int, mutable: Boolean,
    ): PendingIntent {
        val intent = Intent(context, AnswerReceiver::class.java).apply {
            this.action = action
            putExtra(EXTRA_KEY, notice.key)
            putExtra(EXTRA_ID, tag)
            putExtra(EXTRA_INDEX, index)
            notice.runner?.let { putExtra(EXTRA_RUNNER, it) }
            putStringArrayListExtra(EXTRA_OPTION_LIST, ArrayList(notice.options))
        }
        // A typed answer's intent has to be mutable: the system fills the text in.
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or
            (if (mutable) PendingIntent.FLAG_MUTABLE else PendingIntent.FLAG_IMMUTABLE)
        return PendingIntent.getBroadcast(context, "$tag:$action:$index".hashCode(), intent, flags)
    }

    /**
     * Say an answer wasn't sent, replacing the decision's card, with a tap
     * that opens the task to answer it there.
     */
    fun couldNotAnswer(context: Context, answer: TaskAnswer, why: String) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) return
        val tag = answer.noticeId ?: "task:${answer.key}"
        val open = Intent(context, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            putExtra(Notifier.PUSH_EXTRA_KIND, KIND_TASK)
            putExtra(Notifier.PUSH_EXTRA_TASK, answer.key)
            answer.runner?.let { putExtra(Notifier.PUSH_EXTRA_RUNNER, it) }
        }
        val card = NotificationCompat.Builder(context, channelFor("decision"))
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Couldn’t send your answer")
            .setContentText(why)
            .setStyle(NotificationCompat.BigTextStyle().bigText(why))
            .setAutoCancel(true)
            .setGroup(tag)
            .setContentIntent(
                PendingIntent.getActivity(
                    context, tag.hashCode(), open,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                )
            )
            .build()
        runCatching { NotificationManagerCompat.from(context).notify(tag, 0, card) }
    }
}

/** One answer from a decision's card. */
data class TaskAnswer(val key: String, val runner: String?, val noticeId: String?, val body: String)

/**
 * Answers from decision cards, waiting for a runner to send them through.
 *
 * Claimed once per notice: a second tap, or a second answer to the same card,
 * sends nothing. Released when a send didn't land, so a later tap can try
 * again. One process-wide desk, [shared], which [AnswerReceiver] files into
 * and the app's model sends from while it runs.
 */
class TaskAnswers {
    private val claimed = ConcurrentHashMap.newKeySet<String>()
    private val _pending = MutableStateFlow<List<TaskAnswer>>(emptyList())
    val pending: StateFlow<List<TaskAnswer>> = _pending.asStateFlow()

    private fun claimOf(answer: TaskAnswer) = "${answer.noticeId ?: answer.key}\u001f${answer.runner.orEmpty()}"

    /** File [answer], or false when this notice was already answered from here. */
    fun submit(answer: TaskAnswer): Boolean {
        if (!claimed.add(claimOf(answer))) return false
        _pending.value = _pending.value + answer
        return true
    }

    /**
     * Send each pending answer once through [send], which answers null when it
     * landed or the sentence for why it didn't. A refused answer is released
     * and handed to [refused].
     */
    suspend fun deliver(send: suspend (TaskAnswer) -> String?) = deliver(send) { _, _ -> }

    /** [deliver], saying which answers were refused and why. */
    suspend fun deliver(
        send: suspend (TaskAnswer) -> String?,
        refused: (TaskAnswer, String) -> Unit,
    ) {
        val taken = _pending.value
        if (taken.isEmpty()) return
        _pending.value = _pending.value - taken.toSet()
        for (answer in taken) {
            val why = send(answer) ?: continue
            claimed.remove(claimOf(answer))
            refused(answer, why)
        }
    }

    /** Whether something in this process is sending answers (`AppModel`). */
    @Volatile
    var attached: Boolean = false

    companion object {
        val shared = TaskAnswers()
    }
}
