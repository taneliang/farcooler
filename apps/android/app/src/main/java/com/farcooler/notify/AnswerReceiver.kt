package com.farcooler.notify

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.RemoteInput

/**
 * A decision card's answer button (ov-94).
 *
 * Files the answer with [TaskAnswers.shared], which the app's model sends as a
 * `task.note` answer through the runner the task is on, the note the Needs You
 * rows write; the runner then tells the agent waiting on it. Once per card: a
 * second tap sends nothing.
 *
 * The connections live in the app's model, so with the app not running the
 * answer can't go yet: the card says so and opens the task, where the answer
 * is sent as soon as a runner is reached.
 */
class AnswerReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val key = intent.getStringExtra(TaskNotices.EXTRA_KEY) ?: return
        val options = intent.getStringArrayListExtra(TaskNotices.EXTRA_OPTION_LIST).orEmpty()
        val typed = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(TaskNotices.REMOTE_INPUT)
        val body = TaskNotices.answerOf(
            intent.action, intent.getIntExtra(TaskNotices.EXTRA_INDEX, -1), options, typed,
        ) ?: return
        val answer = TaskAnswer(
            key = key,
            runner = intent.getStringExtra(TaskNotices.EXTRA_RUNNER),
            noticeId = intent.getStringExtra(TaskNotices.EXTRA_ID),
            body = body,
        )
        val desk = TaskAnswers.shared
        if (!desk.submit(answer)) return
        val tag = answer.noticeId ?: "task:$key"
        if (desk.attached) {
            // Sent from here on; the card's buttons have done their job.
            runCatching { NotificationManagerCompat.from(context).cancel(tag, 0) }
        } else {
            TaskNotices.couldNotAnswer(context, answer, "Open Far Cooler to send your answer to $key.")
        }
    }
}
